use super::*;
use crate::storage::Storage;
use super_paste_format::*;
use tokio_util::sync::CancellationToken;

fn instant(value: &str) -> i64 { DateTime::parse_from_rfc3339(value).unwrap().timestamp_millis() }
fn date(value: &str) -> NaiveDate { NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap() }

struct Directory(std::path::PathBuf);
impl Drop for Directory {
    fn drop(&mut self) { std::fs::remove_dir_all(&self.0).expect("remove isolated calendar database"); }
}

struct Day { date: &'static str, start: &'static str, end: &'static str, buckets: usize }

#[test]
fn calendar_and_stored_statistics_follow_real_local_transitions() {
    const CHILD: &str = "SUPER_PASTE_CALENDAR_TEST_ZONE";
    let Ok(zone) = std::env::var(CHILD) else {
        // Local caches its timezone. Set TZ before a fresh process starts,
        // without unsafe process-wide environment changes in parallel tests.
        for zone in ["America/New_York", "Australia/Lord_Howe", "America/Sao_Paulo",
            "Pacific/Apia", "Antarctica/Casey", "Atlantic/Azores", "Asia/Kolkata"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "calendar::tests::calendar_and_stored_statistics_follow_real_local_transitions", "--nocapture"])
                .env("TZ", zone).env(CHILD, zone).output().unwrap();
            assert!(output.status.success(), "{zone}: {}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            println!("verified local calendar and SQLite statistics: {zone}");
        }
        return;
    };
    let cases = match zone.as_str() {
        "America/New_York" => vec![
            Day { date: "2026-11-01", start: "2026-11-01T04:00:00Z", end: "2026-11-02T05:00:00Z", buckets: 25 },
            Day { date: "2026-03-08", start: "2026-03-08T05:00:00Z", end: "2026-03-09T04:00:00Z", buckets: 23 },
        ],
        "Australia/Lord_Howe" => vec![
            Day { date: "2026-04-05", start: "2026-04-04T13:00:00Z", end: "2026-04-05T13:30:00Z", buckets: 25 },
            Day { date: "2026-10-04", start: "2026-10-03T13:30:00Z", end: "2026-10-04T13:00:00Z", buckets: 24 },
        ],
        "America/Sao_Paulo" => vec![
            Day { date: "2018-11-04", start: "2018-11-04T03:00:00Z", end: "2018-11-05T02:00:00Z", buckets: 23 },
            Day { date: "2018-02-17", start: "2018-02-17T02:00:00Z", end: "2018-02-18T03:00:00Z", buckets: 25 },
        ],
        "Pacific/Apia" => {
            assert_eq!(day_start(date("2011-12-30")).unwrap(), day_start(date("2011-12-31")).unwrap());
            vec![Day { date: "2011-12-29", start: "2011-12-29T10:00:00Z", end: "2011-12-30T10:00:00Z", buckets: 24 }]
        }
        "Antarctica/Casey" => vec![Day { date: "2018-03-11", start: "2018-03-10T13:00:00Z", end: "2018-03-11T16:00:00Z", buckets: 27 }],
        "Atlantic/Azores" => vec![Day { date: "2026-10-25", start: "2026-10-25T00:00:00Z", end: "2026-10-26T01:00:00Z", buckets: 25 }],
        "Asia/Kolkata" => vec![Day { date: "2026-11-17", start: "2026-11-16T18:30:00Z", end: "2026-11-17T18:30:00Z", buckets: 24 }],
        _ => panic!("unexpected calendar test zone"),
    };

    let directory = Directory(std::env::temp_dir().join(format!("super-paste-rs-calendar-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&directory.0).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    runtime.block_on(async {
        let storage = Storage::open(directory.0.join("calendar.sqlite3")).await.unwrap();
        let cancel = CancellationToken::new();
        for case in cases {
            let (start, end) = (instant(case.start), instant(case.end));
            assert_eq!(day_start(date(case.date)).unwrap(), start);
            assert_eq!(period_start(start + 1_234, StatisticsInterval::Day).unwrap(), start);
            assert_eq!(period_next(start, StatisticsInterval::Day).unwrap(), end);
            let mut count = 0_u64;
            for timestamp in (start..end).step_by(15 * 60 * 1_000) {
                // Samples straddle every real boundary, including repeated hours.
                assert!(storage.record_copy(CopyEvent { timestamp, kind: PrimaryType::Text, source: "calendar fixture".into(), bytes: 3, files: 0 }).await.unwrap());
                assert!(storage.record_ai_usage(AiUsageEntry {
                    id: uuid::Uuid::new_v4(), provider: AiProvider::Openai, base_url: "https://calendar.invalid".into(), model: "fixture".into(), purpose: AiPurpose::Ocr,
                    started_at: timestamp, finished_at: timestamp + 1, duration_ms: 1, success: true,
                    tokens: AiTokens { input_tokens: Some(1), output_tokens: Some(2), total_tokens: Some(3), ..Default::default() },
                }).await.unwrap());
                count += 1;
            }
            let query = |interval| CopyStatsQuery { from: Some(start), to: Some(end), interval, ..Default::default() };
            let hourly = storage.copy_stats(query(StatisticsInterval::Hour), end, &cancel).await.unwrap();
            assert_eq!(hourly.totals.copies, count);
            assert_eq!(hourly.series.len(), case.buckets, "{}", case.date);
            assert_eq!(hourly.series.iter().map(|point| point.totals.copies).sum::<u64>(), count);
            for (index, point) in hourly.series.iter().enumerate() {
                let next = hourly.series.get(index + 1).map_or(end, |point| point.timestamp);
                assert!(point.timestamp < next);
                assert_eq!(point.totals.copies, ((next - point.timestamp) / (15 * 60 * 1_000)) as u64);
                assert_eq!(period_start(point.timestamp + 1_234, StatisticsInterval::Hour).unwrap(), point.timestamp);
                let usage = storage.ai_usage(AiUsageQuery { from: Some(point.timestamp), to: Some(point.timestamp) }, &cancel).await.unwrap();
                assert_eq!(usage.totals.requests, point.totals.copies, "inclusive AI range must end at this actual bucket boundary");
            }
            let daily = storage.copy_stats(query(StatisticsInterval::Day), end, &cancel).await.unwrap();
            assert_eq!(daily.series.len(), 1);
            assert_eq!(daily.series[0].totals.copies, count);
            let monthly = storage.copy_stats(query(StatisticsInterval::Month), end, &cancel).await.unwrap();
            assert_eq!(monthly.series.len(), 1);
            assert_eq!(monthly.series[0].totals.copies, count);
            let usage = storage.ai_usage(AiUsageQuery { from: Some(start), to: Some(end - 1) }, &cancel).await.unwrap();
            assert_eq!(usage.totals.requests, count);
            assert_eq!(usage.totals.total_tokens, count * 3);
            assert_eq!(usage.days.len(), 1);
            assert_eq!(usage.days[0].day, case.date);
        }
    });
}
