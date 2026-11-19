//! Typed CSS counter values; shared by every native rich-text consumer.
use std::sync::Arc;
use cssparser::{ParseError, Parser, Token};

mod complex;

// CSS Counter Styles permits fallback for representations exceeding 60 code
// points. Support 150, with no allocation or loop proportional to the ordinal.
const MAX_COUNTER_CHARS: usize = 150;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum System { Numeric, Alphabetic, Cyclic, Symbolic, Fixed, Additive, DisclosureOpen, DisclosureClosed, Cjk(complex::Cjk), EthiopicNumeric }

#[derive(Clone, Copy)]
struct Definition {
    system: System, symbols: &'static [&'static str], additive: &'static [(u64, &'static str)],
    range: (i64, i64), negative: &'static str, suffix: &'static str, pad: (usize, &'static str), fallback_cjk: bool,
}

/// Shared authored symbols are retained once, never expanded per list item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnonymousSymbols { system: System, symbols: Arc<[Arc<str>]> }

/// Literal markers remain shared through the native UI boundary.
pub enum MarkerText<'a> { Static(&'static str), Shared(&'a Arc<str>), Generated(String) }
impl AsRef<str> for MarkerText<'_> {
    fn as_ref(&self) -> &str { match self { Self::Static(value) => value, Self::Shared(value) => value, Self::Generated(value) => value } }
}

macro_rules! predefined {
    ($($variant:ident => ($name:literal, $definition:expr),)*) => {
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub enum ListStyle { None, Literal(Arc<str>), Symbols(Arc<AnonymousSymbols>), $($variant,)* }
        impl ListStyle {
            pub(super) fn named(value: &str) -> Option<Self> {
                Some(match value.to_ascii_lowercase().as_str() {
                    "none" => Self::None, "default" => return None,
                    $($name => Self::$variant,)*
                    // The sanitizer retains no @counter-style registry. CSS
                    // requires decimal for unresolved custom counter names.
                    _ => Self::Decimal,
                })
            }
            fn definition(&self) -> Option<&'static Definition> {
                match self { $(Self::$variant => Some(&$definition),)* _ => None }
            }
        }
    }
}
include!("lists/data.rs");

impl ListStyle {
    pub(super) fn non_ident<'i>(input: &mut Parser<'i, '_>) -> Result<Self, ParseError<'i, ()>> {
        match input.next()?.clone() {
            Token::QuotedString(value) => Ok(Self::Literal(Arc::from(value.as_ref()))),
            Token::Function(name) if name.eq_ignore_ascii_case("symbols") => input.parse_nested_block(|input| {
                let system = if let Ok(name) = input.try_parse(|input| input.expect_ident_cloned()) {
                    match name.to_ascii_lowercase().as_str() {
                        "numeric" => System::Numeric, "alphabetic" => System::Alphabetic,
                        "cyclic" => System::Cyclic, "symbolic" => System::Symbolic, "fixed" => System::Fixed,
                        _ => return Err(input.new_custom_error(())),
                    }
                } else { System::Symbolic };
                let mut symbols = Vec::new();
                while !input.is_exhausted() { symbols.push(Arc::from(input.expect_string()?.as_ref())); }
                let minimum = if matches!(system, System::Numeric | System::Alphabetic) { 2 } else { 1 };
                if symbols.len() < minimum { return Err(input.new_custom_error(())); }
                Ok(Self::Symbols(Arc::new(AnonymousSymbols { system, symbols: symbols.into() })))
            }),
            _ => Err(input.new_custom_error(())),
        }
    }

    pub(super) fn allocated_weight(&self) -> u64 {
        match self {
            Self::Literal(value) => value.len() as u64 + 16,
            Self::Symbols(value) => (std::mem::size_of::<AnonymousSymbols>() + 32
                + value.symbols.len() * std::mem::size_of::<Arc<str>>()
                + value.symbols.iter().map(|symbol| symbol.len() + 16).sum::<usize>()) as u64,
            _ => 0,
        }
    }

    /// The marker is presentation only; it is not appended to document text.
    /// Fallback replaces the representation, preserving the specified suffix.
    pub fn marker(&self, ordinal: i64, rtl: bool) -> MarkerText<'_> {
        match self {
            Self::None => MarkerText::Static(""),
            Self::Disc => MarkerText::Static("• "),
            Self::Circle => MarkerText::Static("◦ "),
            Self::Square => MarkerText::Static("▪ "),
            Self::Literal(value) => MarkerText::Shared(value),
            Self::Symbols(value) => {
                let symbols = Symbols::Shared(&value.symbols);
                let mut text = positional(value.system, symbols, ordinal)
                    .map(|text| signed(value.system, ordinal, "-", text))
                    .filter(within_limit).unwrap_or_else(|| ordinal.to_string());
                text.push(' ');
                MarkerText::Generated(text)
            },
            _ => {
                let definition = self.definition().expect("predefined counter definition");
                let text = definition.representation(ordinal, rtl).unwrap_or_else(|| {
                    if definition.fallback_cjk && ordinal >= 0 {
                        positional(System::Numeric, Symbols::Static(ListStyle::CjkDecimal.definition().unwrap().symbols), ordinal)
                            .expect("CJK decimal represents nonnegative integers")
                    } else { ordinal.to_string() }
                });
                let mut label = text;
                label.push_str(definition.suffix);
                MarkerText::Generated(label)
            },
        }
    }
}

fn within_limit(text: &String) -> bool { text.chars().take(MAX_COUNTER_CHARS + 1).count() <= MAX_COUNTER_CHARS }

fn signed(system: System, ordinal: i64, negative: &str, text: String) -> String {
    if ordinal < 0 && matches!(system, System::Numeric | System::Additive | System::Cjk(_)) { format!("{negative}{text}") }
    else { text }
}

impl Definition {
    fn representation(&self, ordinal: i64, rtl: bool) -> Option<String> {
        if ordinal < self.range.0 || ordinal > self.range.1 { return None; }
        let mut text = match self.system {
            System::Additive => {
                let mut value = ordinal.unsigned_abs();
                let mut output = String::new();
                if value == 0 { output.push_str(self.additive.iter().find(|(weight, _)| *weight == 0)?.1); }
                for &(weight, symbol) in self.additive {
                    if weight == 0 { continue; }
                    push_repeated(&mut output, symbol, value / weight)?;
                    value %= weight;
                }
                if value != 0 { return None; }
                output
            },
            System::Cjk(style) => complex::cjk(style, ordinal.unsigned_abs()),
            System::EthiopicNumeric => complex::ethiopic(ordinal as u64),
            System::DisclosureOpen => "▾".to_owned(),
            System::DisclosureClosed => if rtl { "◂" } else { "▸" }.to_owned(),
            system => positional(system, Symbols::Static(self.symbols), ordinal)?,
        };
        let sign_chars = if ordinal < 0 { self.negative.chars().count() } else { 0 };
        let padding = self.pad.0.saturating_sub(text.chars().count() + sign_chars);
        if padding != 0 {
            let mut padded = String::new();
            push_repeated(&mut padded, self.pad.1, padding as u64)?;
            padded.push_str(&text); text = padded;
        }
        let text = signed(self.system, ordinal, self.negative, text);
        within_limit(&text).then_some(text)
    }
}

#[derive(Clone, Copy)]
enum Symbols<'a> { Static(&'a [&'static str]), Shared(&'a [Arc<str>]) }
impl<'a> Symbols<'a> {
    fn len(self) -> usize { match self { Self::Static(value) => value.len(), Self::Shared(value) => value.len() } }
    fn get(self, index: usize) -> &'a str { match self { Self::Static(value) => value[index], Self::Shared(value) => &value[index] } }
}

fn push_repeated(output: &mut String, symbol: &str, repetitions: u64) -> Option<()> {
    if symbol.is_empty() || repetitions == 0 { return Some(()); }
    let length = symbol.chars().take(MAX_COUNTER_CHARS + 1).count();
    let available = MAX_COUNTER_CHARS.saturating_sub(output.chars().count());
    if repetitions > (available / length) as u64 { return None; }
    for _ in 0..repetitions { output.push_str(symbol); }
    Some(())
}

fn positional(system: System, symbols: Symbols<'_>, ordinal: i64) -> Option<String> {
    let base = symbols.len() as u64;
    let mut output = String::new();
    match system {
        System::Cyclic => {
            let index = ((ordinal.rem_euclid(base as i64) + base as i64 - 1) % base as i64) as usize;
            push_repeated(&mut output, symbols.get(index), 1)?;
        },
        System::Fixed => {
            if ordinal < 1 || ordinal as u64 > base { return None; }
            push_repeated(&mut output, symbols.get(ordinal as usize - 1), 1)?;
        },
        System::Symbolic => {
            if ordinal < 1 { return None; }
            let value = ordinal as u64 - 1;
            push_repeated(&mut output, symbols.get((value % base) as usize), value / base + 1)?;
        },
        System::Alphabetic | System::Numeric => {
            if system == System::Alphabetic && ordinal < 1 { return None; }
            let mut digits = [0usize; 64];
            let mut cursor = digits.len();
            let mut value = ordinal.unsigned_abs();
            loop {
                if system == System::Alphabetic { value -= 1; }
                cursor -= 1; digits[cursor] = (value % base) as usize; value /= base;
                if value == 0 { break; }
            }
            for &index in &digits[cursor..] { push_repeated(&mut output, symbols.get(index), 1)?; }
        },
        _ => unreachable!("non-positional counter system"),
    }
    Some(output)
}
