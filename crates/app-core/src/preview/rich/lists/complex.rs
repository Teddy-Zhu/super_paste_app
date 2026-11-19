// CJK/Ethiopic algorithms adapted from WebKit CSSRegisteredCounterStyle.cpp.
// Copyright (C) 2022-2024 Apple Inc., BSD-2-Clause; full notice is distributed
// in licenses/webkit-counter-styles-formatting.txt. See the retained upstream
// snapshot and provenance in vendor/css-counter-styles/UPSTREAM.md.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cjk { SimplifiedInformal, SimplifiedFormal, TraditionalInformal, TraditionalFormal, JapaneseInformal, JapaneseFormal, KoreanHangul, KoreanInformal, KoreanFormal }

impl Cjk {
    fn chinese(self) -> bool { matches!(self, Self::SimplifiedInformal | Self::SimplifiedFormal | Self::TraditionalInformal | Self::TraditionalFormal) }
    fn korean(self) -> bool { matches!(self, Self::KoreanHangul | Self::KoreanInformal | Self::KoreanFormal) }
    fn informal(self) -> bool { matches!(self, Self::SimplifiedInformal | Self::TraditionalInformal | Self::JapaneseInformal | Self::KoreanInformal) }
}

pub(super) fn cjk(style: Cjk, mut number: u64) -> String {
    let (digits, units, groups) = match style {
        Cjk::SimplifiedInformal => ("零一二三四五六七八九", "十百千", ["万", "亿", "万亿"]),
        Cjk::SimplifiedFormal => ("零壹贰叁肆伍陆柒捌玖", "拾佰仟", ["万", "亿", "万亿"]),
        Cjk::TraditionalInformal | Cjk::KoreanInformal => ("零一二三四五六七八九", "十百千", ["萬", "億", "兆"]),
        Cjk::TraditionalFormal => ("零壹貳參肆伍陸柒捌玖", "拾佰仟", ["萬", "億", "兆"]),
        Cjk::JapaneseInformal => ("〇一二三四五六七八九", "十百千", ["万", "億", "兆"]),
        Cjk::JapaneseFormal => ("零壱弐参四伍六七八九", "拾百阡", ["萬", "億", "兆"]),
        Cjk::KoreanHangul => ("영일이삼사오육칠팔구", "십백천", ["만", "억", "조"]),
        Cjk::KoreanFormal => ("零壹貳參四五六七八九", "拾百仟", ["萬", "億", "兆"]),
    };
    let mut digit_table = ['\0'; 10];
    for (slot, digit) in digit_table.iter_mut().zip(digits.chars()) { *slot = digit; }
    let mut unit_table = ['\0'; 3];
    for (slot, unit) in unit_table.iter_mut().zip(units.chars()) { *slot = unit; }
    if number == 0 { return digit_table[0].to_string(); }

    // 0 = absent, 1..10 = digits, 11..13 = digit units,
    // 14..16 = group units, 17 = Korean group separator.
    let mut buffer = [0u8; 36];
    let mut has_lower_group = false;
    for index in 0..4 {
        let value = number % 10_000; number /= 10_000;
        let offset = (3 - index) * 9;
        let group = &mut buffer[offset..offset + 9];
        if value != 0 {
            if index != 0 { group[7] = 13 + index as u8; }
            if style.korean() && has_lower_group { group[8] = 17; }
            has_lower_group = true;
        }
        group[6] = 1 + (value % 10) as u8;
        for (power, slot) in [(10, 4), (100, 2), (1000, 0)] {
            if number != 0 || value >= power {
                let digit = (value / power) % 10;
                group[slot] = 1 + digit as u8;
                if digit != 0 { group[slot + 1] = 13 - (slot / 2) as u8; }
            }
        }
        if style.informal() {
            if style.chinese() {
                if (10..=19).contains(&value) { group[4] = 0; }
            } else {
                for slot in [0, 2, 4] { if group[slot] == 2 && group[slot + 1] != 0 { group[slot] = 0; } }
                if style.korean() && index == 1 && value == 1 { group[6] = 0; }
            }
        }
        if style.chinese() {
            if value != 0 {
                for slot in (0..7).rev() {
                    if group[slot] == 0 { continue; }
                    if group[slot] != 1 { break; }
                    group[slot] = 0;
                }
            }
        } else {
            for slot in [0, 2, 4, 6] { if group[slot] == 1 { group[slot] = 0; } }
        }
        if number == 0 { break; }
    }
    let mut compact = [0u8; 36];
    let mut length = 0;
    for value in buffer {
        if value == 0 || (style.chinese() && value == 1 && length != 0 && compact[length - 1] == 1) { continue; }
        compact[length] = value; length += 1;
    }
    if style.chinese() && length != 0 && compact[length - 1] == 1 { length -= 1; }
    let mut output = String::new();
    for &value in &compact[..length] {
        match value {
            1..=10 => output.push(digit_table[value as usize - 1]),
            11..=13 => output.push(unit_table[value as usize - 11]),
            14..=16 => output.push_str(groups[value as usize - 14]),
            17 => output.push(' '),
            _ => unreachable!("CJK abstract symbol"),
        }
    }
    output
}

pub(super) fn ethiopic(mut number: u64) -> String {
    if number == 1 { return "፩".to_owned(); }
    let mut groups = [0u8; 10];
    for group in &mut groups { *group = (number % 100) as u8; number /= 100; }
    let mut output = String::new();
    let mut most_significant = true;
    for (index, &value) in groups.iter().enumerate().rev() {
        let odd = index % 2 == 1;
        if !(value == 1 && (most_significant || odd)) {
            if value / 10 != 0 { output.push(char::from_u32(0x1371 + u32::from(value / 10)).unwrap()); }
            if value % 10 != 0 { output.push(char::from_u32(0x1368 + u32::from(value % 10)).unwrap()); }
        }
        if value != 0 && odd { output.push('፻'); }
        if (value != 0 || !most_significant) && !odd && index != 0 { output.push('፼'); }
        if value != 0 { most_significant = false; }
    }
    output
}
