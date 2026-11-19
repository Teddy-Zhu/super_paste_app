use crate::PrimaryType;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatisticsInterval { Hour, #[default] Day, Month }
#[derive(Clone, Debug, Default)]
pub struct CopyStatsQuery {
    pub from: Option<i64>, pub to: Option<i64>, pub kind: Option<PrimaryType>,
    pub source: Option<String>, pub interval: StatisticsInterval,
}
#[derive(Clone, Debug)]
pub struct CopyEvent { pub timestamp: i64, pub kind: PrimaryType, pub source: String, pub bytes: u64, pub files: u64 }
#[derive(Clone, Debug, Default)]
pub struct CopyTotals {
    pub copies: u64, pub bytes: u64, pub files: u64,
    pub first_copied_at: Option<i64>, pub last_copied_at: Option<i64>,
}
impl CopyTotals {
    pub fn merge(&mut self, other: &Self) {
        self.copies += other.copies; self.bytes += other.bytes; self.files += other.files;
        self.first_copied_at = match (self.first_copied_at, other.first_copied_at) { (Some(a), Some(b)) => Some(a.min(b)), (a, b) => a.or(b) };
        self.last_copied_at = match (self.last_copied_at, other.last_copied_at) { (Some(a), Some(b)) => Some(a.max(b)), (a, b) => a.or(b) };
    }
}
#[derive(Clone, Debug)]
pub struct CopyGroup { pub key: String, pub totals: CopyTotals }
#[derive(Clone, Debug)]
pub struct CopySeriesPoint { pub key: String, pub timestamp: i64, pub totals: CopyTotals }
#[derive(Clone, Debug)]
pub struct CopyStats {
    pub totals: CopyTotals, pub cleared_at: Option<i64>, pub active_days: usize,
    pub types: Vec<CopyGroup>, pub sources: Vec<CopyGroup>, pub available_sources: Vec<String>,
    pub series: Vec<CopySeriesPoint>, pub interval: StatisticsInterval,
}
