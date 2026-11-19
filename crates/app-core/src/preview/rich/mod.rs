mod html;
mod css;
mod rtf;
mod length;
mod math;
mod lists;
pub use length::LengthPercentage;
pub use lists::{ListStyle, MarkerText};
pub use css::{DeferredStyles, ComputedColor};
pub(super) use html::parse as html;
pub(super) use rtf::parse as rtf;

use std::sync::Arc;
use crate::images::EncodedImageInput;
use crate::{Error, Result, resources::{ByteBudget, Reservation, Weighted}};

pub(crate) const MAX_SOURCE_UNITS: usize = 200_000;
pub(crate) fn source_within_limit(source: &str) -> bool {
    source.encode_utf16().take(MAX_SOURCE_UNITS + 1).count() <= MAX_SOURCE_UNITS
}

/// A controlled document shared by cards and details. It contains neither
/// serialized HTML nor GUI types, external URLs, scripts or arbitrary CSS.
#[derive(Clone)]
pub struct Document { pub nodes: Arc<[Node]>, pub text: Arc<str>, bytes: u64, visible_structure: bool,
    style_variables: bool, lease: Option<Arc<Reservation>> }
impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_struct("Document").field("bytes", &self.bytes).field("has_content", &self.has_content()).finish_non_exhaustive() }
}
impl Weighted for Document { fn weight(&self) -> u64 { self.bytes } }
#[derive(Clone, Debug)]
pub enum Node { Text(Arc<str>), Element(Element) }
#[derive(Clone, Debug)]
pub struct Element { pub kind: ElementKind, pub style: Style, pub direction: Option<Direction>, pub title: Option<Arc<str>>, pub children: Arc<[Node]>, first_strong: Option<Direction> }
impl Element {
    /// Computed once from descendant text while finalizing the controlled
    /// document. No strong character means LTR, as with HTML dir=auto.
    pub fn automatic_direction(&self) -> Direction { self.first_strong.unwrap_or(Direction::Ltr) }
}
#[derive(Clone, Debug)]
pub enum ElementKind {
    Paragraph, Container, Span, Bold, Italic, Underline, Strike, Mark, Small, Subscript, Superscript,
    Break, Pre, Code, Quote, UnorderedList, OrderedList { start: i32 }, ListItem,
    Table, TableHead, TableBody, TableFoot, TableRow, Cell { heading: bool, columns: u8, rows: u8 },
    Caption, Rule, Heading(u8), Link, Image { data: Option<Arc<EncodedImageInput>>, alt: Arc<str> },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment { Left, Right, Center, Justify, Start, End, MatchParent }
impl Alignment {
    /// Match-parent resolves logical edges against the computing parent's
    /// direction. Ordinary inheritance retains Start/End for the child's
    /// direction; the document root has no parent and computes to Start.
    /// A supplied parent's alignment must already be computed.
    pub fn resolve_match_parent(self, parent: Option<(Self, bool)>) -> Self {
        if self != Self::MatchParent { return self; }
        match parent {
            None => Self::Start,
            Some((Self::Start, false) | (Self::End, true)) => Self::Left,
            Some((Self::Start, true) | (Self::End, false)) => Self::Right,
            Some((Self::MatchParent, _)) => unreachable!("parent text alignment must already be computed"),
            Some((alignment, _)) => alignment,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum VerticalAlignment { Baseline, Subscript, Superscript, Top, Middle, Bottom, TextTop, TextBottom, Offset(Arc<LengthPercentage>) }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhiteSpace { Normal, NoWrap, Pre, PreWrap, PreLine, PreLineNoWrap, BreakSpaces, BreakSpacesNoWrap }
impl WhiteSpace {
    pub fn preserves_spaces(self) -> bool {
        matches!(self, Self::Pre | Self::PreWrap | Self::BreakSpaces | Self::BreakSpacesNoWrap)
    }
    pub fn preserves_breaks(self) -> bool {
        !matches!(self, Self::Normal | Self::NoWrap)
    }
    pub fn allows_wrap(self) -> bool {
        matches!(self, Self::Normal | Self::PreWrap | Self::PreLine | Self::BreakSpaces)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction { Ltr, Rtl, Auto }
impl Direction {
    /// HTML text-node directionality differs from UAX #9 paragraph detection:
    /// it examines L/AL/R characters even inside Unicode isolate controls.
    /// https://html.spec.whatwg.org/multipage/dom.html#text-node-directionality
    pub fn first_strong(text: &str) -> Option<Self> {
        text.chars().find_map(Self::strong_character)
    }
    fn strong_character(character: char) -> Option<Self> {
        match unicode_bidi::bidi_class(character) {
            unicode_bidi::BidiClass::L => Some(Self::Ltr),
            unicode_bidi::BidiClass::R | unicode_bidi::BidiClass::AL => Some(Self::Rtl),
            _ => None,
        }
    }
}
/// Authored weight is resolved against the parent, before native layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Weight { Absolute(f32), Bolder, Lighter }

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum FontStyle {
    #[default] Normal,
    Italic,
    Oblique,
    /// Preserve the requested degrees separately from the platform's bare
    /// oblique face. An adapter must honor them or reject this native path.
    ObliqueAngle(f32),
}
impl Weight {
    /// CSS Fonts 4 relative-weight table, including its unchanged outer ranges.
    /// https://drafts.csswg.org/css-fonts-4/#relative-weights
    pub fn resolve(self, inherited: f32) -> f32 {
        match self {
            Self::Absolute(value) => value,
            Self::Bolder if inherited < 350.0 => 400.0,
            Self::Bolder if inherited < 550.0 => 700.0,
            Self::Bolder if inherited < 900.0 => 900.0,
            Self::Bolder => inherited,
            Self::Lighter if inherited < 100.0 => inherited,
            Self::Lighter if inherited < 550.0 => 100.0,
            Self::Lighter if inherited < 750.0 => 400.0,
            Self::Lighter => 700.0,
        }
    }
}
/// Properties retained by the clipboard preview's CSS whitelist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StyleProperty { Color, Background, Weight, FontStyle, Decoration, Alignment, Vertical, Space, List }
impl StyleProperty {
    pub fn inherited(self) -> bool { matches!(self, Self::Color | Self::Weight | Self::FontStyle | Self::Alignment | Self::Space | Self::List) }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StyleReset { Inherit = 1, Initial, Revert, RevertLayer }
/// Three bits per whitelisted property; zero means an ordinary declaration.
/// This survives parsing without retaining CSS strings or allocating a map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StyleResets(u32);
impl StyleResets {
    pub fn get(self, property: StyleProperty) -> Option<StyleReset> {
        match (self.0 >> (property as u32 * 3)) & 7 {
            0 => None, 1 => Some(StyleReset::Inherit), 2 => Some(StyleReset::Initial),
            3 => Some(StyleReset::Revert), 4 => Some(StyleReset::RevertLayer),
            _ => unreachable!("invalid controlled style reset"),
        }
    }
    pub(super) fn set(&mut self, property: StyleProperty, reset: Option<StyleReset>) {
        let shift = property as u32 * 3;
        self.0 = (self.0 & !(7 << shift)) | ((reset.map_or(0, |reset| reset as u32)) << shift);
    }
}
/// Preserve currentcolor until the consuming property resolves its used value.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum CssColor { #[default] Current, Rgba([u8; 4]), Computed(Arc<ComputedColor>) }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecorationPattern { #[default] Solid, Double, Dotted, Dashed, Wavy }
#[derive(Clone, Debug, Default, PartialEq)]
pub enum DecorationThickness {
    #[default] Auto,
    FromFont,
    Length { value: f32, unit: LengthUnit },
    /// A fraction of the decorating element's font size, not its descendants'.
    Percentage(f32),
    Calculation(Arc<LengthPercentage>),
}
impl DecorationThickness {
    /// Absolute CSS lengths use 96 px per inch. Relative lengths retain their
    /// unit until a renderer has the decorating element's typography/context.
    pub fn absolute_pixels(&self) -> Option<f64> {
        let Self::Length { value, unit } = self else { return None; };
        unit.absolute_pixels(f64::from(*value))
    }
}
macro_rules! length_units {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u8)]
        pub enum LengthUnit { $($variant),+ }
        impl LengthUnit {
            pub const COUNT: usize = [$(Self::$variant),+].len();
            pub fn parse(value: &str) -> Option<Self> {
                Some(match value { $($name => Self::$variant,)+ _ => return None })
            }
        }
    };
}
length_units! {
    Px => "px", Cm => "cm", Mm => "mm", Q => "q", In => "in", Pc => "pc", Pt => "pt",
    Em => "em", Ex => "ex", Ch => "ch", Rem => "rem", Cap => "cap", Ic => "ic",
    Lh => "lh", Rlh => "rlh", Rex => "rex", Rch => "rch", Rcap => "rcap", Ric => "ric",
    Vw => "vw", Vh => "vh", Vi => "vi", Vb => "vb", Vmin => "vmin", Vmax => "vmax",
    Svw => "svw", Svh => "svh", Svi => "svi", Svb => "svb", Svmin => "svmin", Svmax => "svmax",
    Lvw => "lvw", Lvh => "lvh", Lvi => "lvi", Lvb => "lvb", Lvmin => "lvmin", Lvmax => "lvmax",
    Dvw => "dvw", Dvh => "dvh", Dvi => "dvi", Dvb => "dvb", Dvmin => "dvmin", Dvmax => "dvmax",
    Cqw => "cqw", Cqh => "cqh", Cqi => "cqi", Cqb => "cqb", Cqmin => "cqmin", Cqmax => "cqmax",
}
impl LengthUnit {
    /// Absolute CSS conversions share the same 96 px/in definition across
    /// computed calculations and decoration metrics.
    pub fn absolute_pixels(self, value: f64) -> Option<f64> {
        let scale = match self {
            Self::Px => 1.0, Self::In => 96.0,
            Self::Cm => 96.0 / 2.54, Self::Mm => 96.0 / 25.4,
            Self::Q => 96.0 / 101.6, Self::Pt => 96.0 / 72.0,
            Self::Pc => 16.0, _ => return None,
        };
        let pixels = value * scale;
        pixels.is_finite().then_some(pixels)
    }
}
/// The shorthand resets all components together. Keeping one value also
/// preserves explicit inherit/currentcolor without per-component allocations.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decoration {
    pub underline: bool, pub overline: bool, pub strike: bool, pub blink: bool,
    pub pattern: DecorationPattern, pub color: CssColor, pub thickness: DecorationThickness,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub variable_styles: Option<Arc<DeferredStyles>>,
    pub foreground: Option<CssColor>, pub background: Option<CssColor>, pub weight: Option<Weight>,
    pub font_style: Option<FontStyle>, pub decoration: Option<Decoration>,
    pub alignment: Option<Alignment>, pub vertical: Option<VerticalAlignment>, pub white_space: Option<WhiteSpace>, pub list_style: Option<ListStyle>, pub resets: StyleResets,
}
impl Document {
    pub fn has_content(&self) -> bool { !self.text.is_empty() || self.visible_structure }
    pub fn weight(&self) -> u64 { self.bytes }
    pub fn has_style_variables(&self) -> bool { self.style_variables }
    pub(super) fn charge(&mut self, memory: &ByteBudget) -> Result<()> { self.lease = Some(Arc::new(memory.reserve(self.bytes)?)); Ok(()) }
    pub(super) fn new(mut nodes: Vec<Node>) -> Result<Self> {
        let mut text = SummaryText::default(); let mut count = 0; let mut bytes = std::mem::size_of::<Self>() as u64; let mut visible = false;
        let mut style_variables = false;
        describe(&mut nodes, &mut text, &mut count, &mut bytes, &mut visible, &mut style_variables, 0)?;
        let text: Arc<str> = text.finish().into(); bytes += text.len() as u64;
        Ok(Self { nodes: nodes.into(), text, bytes, visible_structure: visible, style_variables, lease: None })
    }
}

/// Stream the existing summary rules across node boundaries: drop a tab just
/// before a newline, retain at most two consecutive newlines, and trim Unicode
/// whitespace. Only the final summary is built; no full raw/replaced copies.
#[derive(Default)]
struct SummaryText { value: String, pending_tab: bool, newlines: u8 }
impl SummaryText {
    fn push_str(&mut self, value: &str) { for character in value.chars() { self.push(character); } }
    fn text_node(&mut self, value: &str, first_strong: &mut Option<Direction>) {
        for character in value.chars() {
            self.push(character);
            if first_strong.is_none() { *first_strong = Direction::strong_character(character); }
        }
    }
    fn push(&mut self, character: char) {
        if self.pending_tab {
            self.pending_tab = false;
            if character != '\n' { self.emit('\t'); }
        }
        if character == '\t' { self.pending_tab = true; } else { self.emit(character); }
    }
    fn emit(&mut self, character: char) {
        if character == '\n' {
            if self.newlines == 2 { return; }
            self.newlines += 1;
        } else { self.newlines = 0; }
        if !self.value.is_empty() || !character.is_whitespace() { self.value.push(character); }
    }
    fn finish(mut self) -> String {
        // A pending final tab is trimmed too; do not materialize it first.
        self.value.truncate(self.value.trim_end().len()); self.value
    }
}

fn describe(nodes: &mut [Node], text: &mut SummaryText, count: &mut usize, bytes: &mut u64, visible: &mut bool, style_variables: &mut bool, depth: usize) -> Result<Option<Direction>> {
    if depth > 96 { return Err(Error::ResourceLimit("富文本嵌套过深".into())); }
    let mut first_strong = None;
    for node in nodes {
        *count += 1; if *count > 20_000 { return Err(Error::ResourceLimit("富文本结构过于复杂".into())); }
        *bytes += std::mem::size_of::<Node>() as u64;
        match node {
            Node::Text(value) => {
                text.text_node(value, &mut first_strong); *bytes += value.len() as u64;
            },
            Node::Element(element) => {
                for color in [element.style.foreground.as_ref(), element.style.background.as_ref(), element.style.decoration.as_ref().map(|value| &value.color)].into_iter().flatten() {
                    if let CssColor::Computed(value) = color { *bytes += value.weight(); }
                }
                if let Some(values) = &element.style.variable_styles { *style_variables = true; *bytes += values.weight(); }
                *bytes += element.title.as_ref().map_or(0, |value| value.len() as u64);
                if let Some(VerticalAlignment::Offset(value)) = &element.style.vertical { *bytes += value.weight(); }
                if let Some(Decoration { thickness: DecorationThickness::Calculation(value), .. }) = &element.style.decoration { *bytes += value.weight(); }
                if let Some(value) = &element.style.list_style { *bytes += value.allocated_weight(); }
                if let ElementKind::Image { data, alt } = &element.kind { text.push_str(alt); *bytes += data.as_ref().map_or(0, |source| source.bytes().len() as u64 + std::mem::size_of::<EncodedImageInput>() as u64) + alt.len() as u64; *visible = true; }
                let direction = describe(Arc::make_mut(&mut element.children), text, count, bytes, visible, style_variables, depth + 1)?;
                element.first_strong = direction;
                // A child with its own dir does not determine the ancestor's
                // auto direction. Image alt text is likewise not a Text node.
                if first_strong.is_none() && element.direction.is_none() { first_strong = direction; }
                match &element.kind {
                    ElementKind::Break => text.push('\n'),
                    ElementKind::Paragraph | ElementKind::Container | ElementKind::Pre | ElementKind::Quote | ElementKind::ListItem | ElementKind::TableRow | ElementKind::Heading(_) => text.push('\n'),
                    ElementKind::Cell { .. } => text.push('\t'),
                    ElementKind::Table | ElementKind::Rule => *visible = true,
                    _ => {},
                }
            },
        }
    }
    Ok(first_strong)
}
pub(super) fn element(kind: ElementKind, style: Style, children: Vec<Node>) -> Node {
    Node::Element(Element { kind, style, direction: None, title: None, children: children.into(), first_strong: None })
}
