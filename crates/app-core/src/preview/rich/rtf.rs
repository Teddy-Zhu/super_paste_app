use std::{collections::{HashMap, HashSet}, sync::Arc};
use encoding_rs::{Encoding, WINDOWS_1252};
use rtf_grimoire::tokenizer::{Token, read_token};
use crate::{Error, Result};
use super::{Alignment, CssColor, Direction, Document, Element, ElementKind, FontStyle, Node, Style, VerticalAlignment, element};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Destination { Body, Skip, Fonts, Colors, Picture, Lists, Overrides, LegacyList, UnicodeAlternative }
#[derive(Clone)]
struct State {
    destination: Destination, ignorable: bool, unicode_alternative: bool, style: Style,
    alignment: Option<Alignment>, direction: Option<Direction>, unicode_skip: usize,
    font: i32, font_definition: Option<i32>, encoding: &'static Encoding,
    list: Option<i32>, level: usize, in_table: bool, table_depth: usize,
}
impl Default for State {
    fn default() -> Self { Self { destination: Destination::Body, ignorable: false, unicode_alternative: false, style: Style::default(), alignment: None, direction: None, unicode_skip: 1, font: 0, font_definition: None, encoding: WINDOWS_1252, list: None, level: 0, in_table: false, table_depth: 1 } }
}
#[derive(Default)]
struct Paragraph { children: Vec<Node>, alignment: Option<Alignment>, direction: Option<Direction>, list: Option<i32>, level: usize }
#[derive(Clone, Copy)]
struct ListLevel { format: i32, start: i32 }
impl Default for ListLevel { fn default() -> Self { Self { format: 0, start: 1 } } }
struct ListDefinition { depth: usize, id: i32, levels: Vec<ListLevel> }
struct LevelDefinition { depth: usize, value: ListLevel }
struct Override { depth: usize, list: i32, number: i32, starts: HashMap<usize, i32>, level: usize, next_level: usize }
enum Block { Plain(Node), Listed { number: i32, level: usize, format: i32, start: i32, child: Node } }
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Merge { #[default] None, Start, Continue }
#[derive(Clone, Default)]
struct CellSpec { horizontal: Merge, vertical: Merge, style: Style }
// Keep the parser's cells mutable until all row/column merges are resolved.
// Publishing Arc children earlier would clone the accumulated prefix at every
// continuation. Each child moves into its final shared document only once.
struct Cell { children: Vec<Node>, style: Style, columns: u8, rows: u8, column: usize, vertical: Merge, keep: bool }
#[derive(Default)]
struct Row { specifications: Vec<CellSpec>, next_specification: CellSpec, cells: Vec<Cell>, blocks: Vec<Block>, column: usize }
struct ParentTable { depth: usize, row: Option<Row>, rows: Vec<Vec<Cell>> }
struct Picture { depth: usize, data: Vec<u8>, nibble: Option<u8>, supported: bool }
struct Parser {
    states: Vec<State>, fonts: HashMap<i32, &'static Encoding>, default_font: i32, colors: Vec<Option<[u8; 4]>>,
    color: [u8; 3], color_set: bool, bytes: Vec<u8>, run: String, run_style: Option<Style>, pending_high: Option<u16>, fallback: usize,
    paragraph: Paragraph, blocks: Vec<Block>, row: Option<Row>, rows: Vec<Vec<Cell>>, table_depth: usize, parent_tables: Vec<ParentTable>, picture: Option<Picture>,
    lists: HashMap<i32, Vec<ListLevel>>, overrides: HashMap<i32, (i32, HashMap<usize, i32>)>,
    defining_list: Option<ListDefinition>, defining_level: Option<LevelDefinition>, defining_override: Option<Override>,
    counters: HashMap<(i32, usize), i32>, seen_header: bool, node_count: usize,
}
pub(crate) fn parse(source: &str) -> Result<Document> {
    let source = source.trim_start_matches('\u{feff}').trim_start();
    if !source.starts_with("{\\rtf") { return Err(invalid()); }
    if !super::source_within_limit(source) { return Err(Error::ResourceLimit("富文本超过排版预览上限".into())); }
    let mut parser = Parser { states: vec![State::default()], fonts: HashMap::new(), default_font: 0, colors: Vec::new(), color: [0; 3], color_set: false, bytes: Vec::new(), run: String::new(), run_style: None, pending_high: None, fallback: 0, paragraph: Paragraph::default(), blocks: Vec::new(), row: None, rows: Vec::new(), table_depth: 1, parent_tables: Vec::new(), picture: None, lists: HashMap::new(), overrides: HashMap::new(), defining_list: None, defining_level: None, defining_override: None, counters: HashMap::new(), seen_header: false, node_count: 0 };
    let mut input = source.as_bytes(); let mut tokens = 0;
    while !input.is_empty() {
        let (remaining, token) = read_token(input).map_err(|_| invalid())?;
        if remaining.len() >= input.len() { return Err(invalid()); }
        input = remaining; tokens += 1;
        if tokens > 200_000 { return Err(Error::ResourceLimit("RTF 结构过于复杂".into())); }
        parser.token(token)?;
    }
    if parser.states.len() != 1 || !parser.seen_header || parser.picture.is_some() { return Err(invalid()); }
    parser.flush_bytes()?;
    if parser.pending_high.take().is_some() { parser.emit("\u{fffd}")?; }
    parser.change_table_depth(1)?; parser.paragraph(false)?; if parser.row.is_some() { return Err(invalid()); } parser.flush_table()?;
    Document::new(blocks(parser.blocks))
}
fn invalid() -> Error { Error::Invalid("RTF 文档无法解析".into()) }
impl Parser {
    fn state(&self) -> &State { self.states.last().expect("RTF root state exists") }
    fn state_mut(&mut self) -> &mut State { self.states.last_mut().expect("RTF root state exists") }
    fn token(&mut self, token: Token) -> Result<()> {
        // Unicode fallback counts source bytes and complete control tokens,
        // including an entire binary token. A group delimiter ends the run
        // before the delimiter is processed; skipped controls have no effects.
        if self.fallback > 0 {
            match &token {
                Token::StartGroup | Token::EndGroup => self.fallback = 0,
                Token::ControlWord { .. } | Token::ControlSymbol(_) | Token::ControlBin(_) => {
                    self.fallback -= 1; return Ok(());
                },
                Token::Text(_) | Token::Newline(_) => {},
            }
        }
        match token {
            Token::StartGroup => {
                self.flush_bytes()?;
                if self.states.len() > 96 { return Err(Error::ResourceLimit("RTF 嵌套过深".into())); }
                let mut state = self.state().clone(); state.ignorable = false;
                if state.destination == Destination::UnicodeAlternative { state.destination = Destination::Skip; state.unicode_alternative = true; }
                self.states.push(state);
            },
            Token::EndGroup => {
                self.flush_bytes()?; let depth = self.states.len(); if depth <= 1 { return Err(invalid()); }
                if self.picture.as_ref().is_some_and(|picture| picture.depth == depth) {
                    let picture = self.picture.take().ok_or_else(invalid)?;
                    if picture.nibble.is_some() { return Err(invalid()); }
                    let data = picture.supported.then(|| crate::images::EncodedImageInput::new(bytes::Bytes::from(picture.data)));
                    self.push(element(ElementKind::Image { data, alt: Arc::default() }, self.state().style.clone(), Vec::new()))?;
                }
                if self.defining_level.as_ref().is_some_and(|level| level.depth == depth) {
                    let level = self.defining_level.take().ok_or_else(invalid)?;
                    if let Some(list) = self.defining_list.as_mut() { if list.levels.len() >= 9 { return Err(invalid()); } list.levels.push(level.value); }
                }
                if self.defining_list.as_ref().is_some_and(|list| list.depth == depth) { let list = self.defining_list.take().ok_or_else(invalid)?; self.lists.insert(list.id, list.levels); }
                if self.defining_override.as_ref().is_some_and(|value| value.depth == depth) { let value = self.defining_override.take().ok_or_else(invalid)?; self.overrides.insert(value.number, (value.list, value.starts)); }
                self.states.pop();
                let table_depth = self.state().table_depth; if table_depth != self.table_depth { self.change_table_depth(table_depth)?; }
            },
            Token::ControlWord { name, arg } => { if name != "'" { self.flush_bytes()?; } self.word(&name, arg)?; },
            Token::ControlSymbol(symbol) => {
                if symbol == '*' { self.state_mut().ignorable = true; }
                else if self.state().destination == Destination::Body {
                    self.flush_bytes()?; match symbol { '\\' => self.emit("\\")?, '{' => self.emit("{")?, '}' => self.emit("}")?, '~' => self.emit("\u{a0}")?, '-' => self.emit("\u{ad}")?, '_' => self.emit("\u{2011}")?, '\r' | '\n' => self.paragraph(true)?, _ => {} }
                }
            },
            Token::Text(bytes) => match self.state().destination {
                Destination::Colors => { for byte in bytes { if byte == b';' { self.colors.push(self.color_set.then_some([self.color[0], self.color[1], self.color[2], 255])); self.color = [0; 3]; self.color_set = false; } } },
                Destination::Picture => { if let Some(picture) = &mut self.picture { for byte in bytes { if byte.is_ascii_whitespace() { continue; } let value = (byte as char).to_digit(16).ok_or_else(invalid)? as u8; if let Some(high) = picture.nibble.take() { picture.data.push(high * 16 + value); } else { picture.nibble = Some(value); } } } },
                Destination::Body => {
                    let skip = self.fallback.min(bytes.len()); self.fallback -= skip;
                    let text = std::str::from_utf8(&bytes[skip..]).map_err(|_| invalid())?;
                    if text.is_ascii() { self.bytes.extend_from_slice(text.as_bytes()); } else { self.flush_bytes()?; self.emit(text)?; }
                },
                _ => {},
            },
            Token::ControlBin(bytes) => { if self.state().destination == Destination::Picture { if let Some(picture) = &mut self.picture { if picture.nibble.is_some() { return Err(invalid()); } picture.data.extend(bytes); } } },
            Token::Newline(_) => {},
        }
        Ok(())
    }
    fn word(&mut self, name: &str, arg: Option<i32>) -> Result<()> {
        if name == "ud" && self.state().unicode_alternative { self.state_mut().destination = Destination::Body; self.state_mut().ignorable = false; return Ok(()); }
        if self.state().destination == Destination::Skip { return Ok(()); }
        if self.state().ignorable {
            self.state_mut().ignorable = false;
            if !matches!(name, "fonttbl" | "colortbl" | "pict" | "shppict" | "nesttableprops" | "listtable" | "listoverridetable" | "pn") { self.state_mut().destination = Destination::Skip; return Ok(()); }
        }
        let enabled = arg != Some(0);
        match self.state().destination {
            Destination::Colors => { if let Some(index) = match name { "red" => Some(0), "green" => Some(1), "blue" => Some(2), _ => None } { self.color[index] = arg.unwrap_or(0).clamp(0, 255) as u8; self.color_set = true; } return Ok(()); },
            Destination::Fonts => {
                if name == "f" { self.state_mut().font_definition = arg; }
                else if let Some(font) = self.state().font_definition {
                    let encoding = if name == "fcharset" { arg.and_then(charset) } else if name == "cpg" { arg.and_then(codepage) } else { None };
                    if let Some(encoding) = encoding { self.fonts.insert(font, encoding); }
                }
                return Ok(());
            },
            Destination::Picture => { if matches!(name, "pngblip" | "jpegblip") { if let Some(picture) = self.picture.as_mut() { picture.supported = true; } } return Ok(()); },
            Destination::Lists | Destination::Overrides => { self.list_word(name, arg)?; return Ok(()); },
            Destination::LegacyList => {
                let format = match name { "pnlvlblt" => Some(23), "pndec" => Some(0), "pnucrm" => Some(1), "pnlcrm" => Some(2), "pnucltr" => Some(3), "pnlcltr" => Some(4), _ => None };
                if let Some(format) = format { self.lists.entry(-1).or_insert_with(|| vec![ListLevel::default()])[0].format = format; }
                if name == "pnstart" { self.lists.entry(-1).or_insert_with(|| vec![ListLevel::default()])[0].start = arg.unwrap_or(1); }
                return Ok(());
            },
            _ => {},
        }
        match name {
            "rtf" => { if arg != Some(1) { return Err(invalid()); } self.seen_header = true; },
            "fonttbl" => self.state_mut().destination = Destination::Fonts,
            "colortbl" => self.state_mut().destination = Destination::Colors,
            "listtable" => self.state_mut().destination = Destination::Lists,
            "listoverridetable" => self.state_mut().destination = Destination::Overrides,
            "upr" => self.state_mut().destination = Destination::UnicodeAlternative,
            "pict" => { if self.picture.is_some() { return Err(invalid()); } self.state_mut().destination = Destination::Picture; self.picture = Some(Picture { depth: self.states.len(), data: Vec::new(), nibble: None, supported: false }); },
            "pn" => { self.state_mut().destination = Destination::LegacyList; if self.states.len() >= 2 { let parent = self.states.len() - 2; self.states[parent].list = Some(-1); self.paragraph.list = Some(-1); } },
            "stylesheet" | "info" | "generator" | "filetbl" | "revtbl" | "rsidtbl" | "xmlnstbl" | "themedata" | "datastore" | "object" | "objdata" | "fldinst" | "header" | "headerl" | "headerr" | "footer" | "footerl" | "footerr" | "footnote" | "annotation" | "listtext" | "pntext" | "shp" | "nonshppict" => self.state_mut().destination = Destination::Skip,
            "ansicpg" => self.state_mut().encoding = arg.and_then(codepage).ok_or_else(|| Error::Invalid("RTF 字符编码不受支持".into()))?,
            "mac" => self.state_mut().encoding = encoding_rs::MACINTOSH,
            "deff" => { self.default_font = arg.unwrap_or(0); self.state_mut().font = self.default_font; },
            "f" => self.state_mut().font = arg.unwrap_or(self.default_font),
            "uc" => self.state_mut().unicode_skip = usize::try_from(arg.unwrap_or(1)).map_err(|_| invalid())?,
            "u" => { self.unit(arg.ok_or_else(invalid)? as u16)?; self.fallback = self.state().unicode_skip; },
            "'" => self.bytes.push(u8::try_from(arg.ok_or_else(invalid)?).map_err(|_| invalid())?),
            "b" => self.state_mut().style.weight = Some(super::Weight::Absolute(if enabled { 700.0 } else { 400.0 })),
            "i" => self.state_mut().style.font_style = Some(if enabled { FontStyle::Italic } else { FontStyle::Normal }),
            "ul" | "uld" | "uldb" | "uldash" | "ulwave" => self.state_mut().style.decoration.get_or_insert_with(Default::default).underline = enabled,
            "ulnone" => self.state_mut().style.decoration.get_or_insert_with(Default::default).underline = false,
            "strike" | "striked" => self.state_mut().style.decoration.get_or_insert_with(Default::default).strike = enabled,
            "super" => self.state_mut().style.vertical = Some(VerticalAlignment::Superscript),
            "sub" => self.state_mut().style.vertical = Some(VerticalAlignment::Subscript),
            "nosupersub" => self.state_mut().style.vertical = Some(VerticalAlignment::Baseline),
            "cf" => self.state_mut().style.foreground = self.color_at(arg).map(CssColor::Rgba),
            "highlight" | "cb" => self.state_mut().style.background = self.color_at(arg).map(CssColor::Rgba),
            "plain" => { self.state_mut().style = Style::default(); self.state_mut().font = self.default_font; },
            "pard" => { let state = self.state_mut(); state.alignment = None; state.direction = None; state.list = None; state.level = 0; state.in_table = false; },
            "ql" | "qr" | "qc" | "qj" => { let value = match name { "qr" => Alignment::Right, "qc" => Alignment::Center, "qj" => Alignment::Justify, _ => Alignment::Left }; self.state_mut().alignment = Some(value); self.paragraph.alignment = Some(value); },
            "rtlpar" | "ltrpar" => { let value = if name == "rtlpar" { Direction::Rtl } else { Direction::Ltr }; self.state_mut().direction = Some(value); self.paragraph.direction = Some(value); },
            "ls" => { self.state_mut().list = arg; self.paragraph.list = arg; },
            "ilvl" => { let level = usize::try_from(arg.unwrap_or(0)).map_err(|_| invalid())?; if level > 8 { return Err(invalid()); } self.state_mut().level = level; self.paragraph.level = level; },
            "par" | "sect" | "page" => self.paragraph(true)?,
            "line" => self.push(element(ElementKind::Break, self.state().style.clone(), Vec::new()))?,
            "tab" => self.emit("\t")?, "bullet" => self.emit("•")?, "emdash" => self.emit("—")?, "endash" => self.emit("–")?,
            "lquote" => self.emit("‘")?, "rquote" => self.emit("’")?, "ldblquote" => self.emit("“")?, "rdblquote" => self.emit("”")?,
            "intbl" => self.state_mut().in_table = enabled,
            "trowd" => {
                self.paragraph(false)?;
                if let Some(row) = self.row.as_mut() {
                    if self.table_depth == 1 { return Err(invalid()); }
                    // Nested row properties may follow their cell contents.
                    row.specifications.clear(); row.next_specification = CellSpec::default();
                } else { self.row = Some(Row::default()); }
                self.state_mut().in_table = true;
            },
            "clmgf" | "clmrg" => { if let Some(row) = &mut self.row { row.next_specification.horizontal = if name == "clmgf" { Merge::Start } else { Merge::Continue }; } },
            "clvmgf" | "clvmrg" => { if let Some(row) = &mut self.row { row.next_specification.vertical = if name == "clvmgf" { Merge::Start } else { Merge::Continue }; } },
            "clcbpat" => { let color = self.color_at(arg).map(CssColor::Rgba); if let Some(row) = &mut self.row { row.next_specification.style.background = color; } },
            "clvertalt" | "clvertalc" | "clvertalb" => { if let Some(row) = &mut self.row { row.next_specification.style.vertical = Some(match name { "clvertalc" => VerticalAlignment::Middle, "clvertalb" => VerticalAlignment::Bottom, _ => VerticalAlignment::Top }); } },
            "cellx" => { if let Some(row) = &mut self.row { row.specifications.push(std::mem::take(&mut row.next_specification)); } },
            "cell" | "nestcell" => { if name == "nestcell" && self.table_depth < 2 { return Err(invalid()); } self.cell()?; },
            "row" | "nestrow" => { if name == "nestrow" && self.table_depth < 2 { return Err(invalid()); } self.finish_row()?; },
            "itap" => {
                let depth = usize::try_from(arg.unwrap_or(1)).map_err(|_| invalid())?;
                if !(1..=8).contains(&depth) { return Err(invalid()); }
                self.change_table_depth(depth)?; self.state_mut().table_depth = depth;
            },
            _ => {},
        }
        Ok(())
    }
    fn color_at(&self, index: Option<i32>) -> Option<[u8; 4]> { index.and_then(|index| usize::try_from(index).ok()).and_then(|index| self.colors.get(index).copied().flatten()) }
    fn flush_bytes(&mut self) -> Result<()> {
        if self.bytes.is_empty() { return Ok(()); }
        let data = std::mem::take(&mut self.bytes); let encoding = self.fonts.get(&self.state().font).copied().unwrap_or(self.state().encoding);
        let (text, errors) = encoding.decode_without_bom_handling(&data);
        if errors { return Err(Error::Invalid("RTF 文本编码无效".into())); } self.emit(&text)
    }
    fn unit(&mut self, unit: u16) -> Result<()> {
        if let Some(high) = self.pending_high.take() {
            if (0xdc00..=0xdfff).contains(&unit) { return self.emit(&char::from_u32(0x10000 + ((u32::from(high) - 0xd800) << 10) + u32::from(unit) - 0xdc00).ok_or_else(invalid)?.to_string()); }
            self.emit("\u{fffd}")?;
        }
        if (0xd800..=0xdbff).contains(&unit) { self.pending_high = Some(unit); Ok(()) }
        else { self.emit(&char::from_u32(u32::from(unit)).unwrap_or('\u{fffd}').to_string()) }
    }
    fn emit(&mut self, text: &str) -> Result<()> {
        if text.is_empty() { return Ok(()); }
        let style = self.state().style.clone();
        if self.run_style.as_ref().is_some_and(|previous| *previous != style) { self.flush_run()?; }
        self.begin_paragraph(); self.run_style = Some(style);
        if self.pending_high.take().is_some() { self.run.push('\u{fffd}'); }
        self.run.push_str(text); Ok(())
    }
    fn push(&mut self, node: Node) -> Result<()> {
        self.begin_paragraph(); self.flush_run()?; self.push_direct(node)
    }
    fn begin_paragraph(&mut self) {
        if self.table_depth > 1 && self.state().in_table && self.row.is_none() { self.row = Some(Row::default()); }
        if self.paragraph.children.is_empty() && self.run.is_empty() { self.paragraph.alignment = self.state().alignment; self.paragraph.direction = self.state().direction; self.paragraph.list = self.state().list; self.paragraph.level = self.state().level; }
    }
    fn flush_run(&mut self) -> Result<()> {
        if self.run.is_empty() { return Ok(()); }
        let text = std::mem::take(&mut self.run); let style = self.run_style.take().unwrap_or_default();
        self.push_direct(element(ElementKind::Span, style, vec![Node::Text(text.into())]))
    }
    fn push_direct(&mut self, node: Node) -> Result<()> {
        self.node_count += 2; if self.node_count > 20_000 { return Err(Error::ResourceLimit("RTF 结构过于复杂".into())); }
        self.paragraph.children.push(node); Ok(())
    }
    fn paragraph(&mut self, force: bool) -> Result<()> {
        self.begin_paragraph();
        self.flush_run()?;
        if !force && self.paragraph.children.is_empty() { return Ok(()); }
        let paragraph = std::mem::take(&mut self.paragraph);
        let node = Node::Element(Element { kind: ElementKind::Paragraph, style: Style { alignment: paragraph.alignment, ..Style::default() }, direction: paragraph.direction, title: None, children: paragraph.children.into(), first_strong: None });
        let block = if let Some(number) = paragraph.list {
            let (list_id, starts) = self.overrides.get(&number).map(|(id, starts)| (*id, Some(starts))).unwrap_or((number, None));
            let level = self.lists.get(&list_id).and_then(|levels| levels.get(paragraph.level)).copied().unwrap_or_default();
            let start = starts.and_then(|starts| starts.get(&paragraph.level)).copied().unwrap_or(level.start);
            self.counters.retain(|(list, depth), _| *list != number || *depth <= paragraph.level);
            let counter = self.counters.entry((number, paragraph.level)).or_insert(start); let ordinal = *counter; *counter = counter.saturating_add(1);
            Block::Listed { number, level: paragraph.level, format: level.format, start: ordinal, child: node }
        } else { Block::Plain(node) };
        if let Some(row) = self.row.as_mut() { row.blocks.push(block); } else { self.flush_table()?; self.blocks.push(block); }
        Ok(())
    }
    fn cell(&mut self) -> Result<()> {
        self.paragraph(false)?; let row = self.row.as_mut().ok_or_else(invalid)?;
        let children = blocks(std::mem::take(&mut row.blocks));
        row.cells.push(Cell { children, style: Style::default(), columns: 1, rows: 1, column: row.column, vertical: Merge::None, keep: true });
        row.column += 1; Ok(())
    }
    fn finish_row(&mut self) -> Result<()> {
        self.paragraph(false)?;
        while self.row.as_ref().is_some_and(|row| !row.blocks.is_empty() || row.column < row.specifications.len()) { self.cell()?; }
        let row = self.row.take().ok_or_else(invalid)?; let mut cells: Vec<Cell> = Vec::new();
        for mut cell in row.cells {
            let specification = row.specifications.get(cell.column).cloned().unwrap_or_default();
            cell.style = specification.style; cell.vertical = specification.vertical;
            if specification.horizontal == Merge::Continue {
                if let Some(previous) = cells.last_mut() {
                    previous.columns = previous.columns.saturating_add(1).min(20);
                    previous.children.append(&mut cell.children); continue;
                }
            }
            cells.push(cell);
        }
        self.rows.push(cells); let nested = self.table_depth > 1; self.state_mut().in_table = nested; Ok(())
    }
    fn change_table_depth(&mut self, depth: usize) -> Result<()> {
        if depth == self.table_depth { return Ok(()); }
        self.paragraph(false)?;
        if depth > self.table_depth {
            if depth != self.table_depth + 1 || depth > 8 || self.row.is_none() { return Err(invalid()); }
            self.parent_tables.push(ParentTable { depth: self.table_depth, row: self.row.take(), rows: std::mem::take(&mut self.rows) });
            self.table_depth = depth; self.row = Some(Row::default()); return Ok(());
        }
        while depth < self.table_depth {
            if let Some(row) = self.row.take() {
                if !row.cells.is_empty() || !row.blocks.is_empty() || !row.specifications.is_empty() { return Err(invalid()); }
            }
            let table = self.take_table(); let parent = self.parent_tables.pop().ok_or_else(invalid)?;
            self.table_depth = parent.depth; self.row = parent.row; self.rows = parent.rows;
            if let Some(table) = table { self.row.as_mut().ok_or_else(invalid)?.blocks.push(Block::Plain(table)); }
        }
        Ok(())
    }
    fn flush_table(&mut self) -> Result<()> {
        if self.table_depth != 1 { return Err(invalid()); }
        if let Some(table) = self.take_table() { self.blocks.push(Block::Plain(table)); }
        Ok(())
    }
    fn take_table(&mut self) -> Option<Node> {
        if self.rows.is_empty() { return None; }
        let mut rows = std::mem::take(&mut self.rows); let mut active = HashMap::<usize, (usize, usize)>::new();
        for row_index in 0..rows.len() {
            let (previous, current) = rows.split_at_mut(row_index); let mut advanced = HashSet::new(); let mut next = HashMap::new();
            for (index, cell) in current[0].iter_mut().enumerate() {
                let width = usize::from(cell.columns);
                if cell.vertical == Merge::Continue {
                    if let Some(&(start, start_cell)) = active.get(&cell.column) {
                        let first = &mut previous[start][start_cell];
                        if advanced.insert((start, start_cell)) { first.rows = first.rows.saturating_add(1).min(20); }
                        first.children.append(&mut cell.children);
                        cell.keep = false; for column in cell.column..cell.column + width { next.insert(column, (start, start_cell)); } continue;
                    }
                }
                if cell.vertical == Merge::Start { for column in cell.column..cell.column + width { next.insert(column, (row_index, index)); } }
            }
            active = next;
        }
        let rows = rows.into_iter().map(|row| element(ElementKind::TableRow, Style::default(), row.into_iter().filter(|cell| cell.keep)
            .map(|cell| element(ElementKind::Cell { heading: false, columns: cell.columns, rows: cell.rows }, cell.style, cell.children)).collect())).collect();
        Some(element(ElementKind::Table, Style::default(), rows))
    }
    fn list_word(&mut self, name: &str, arg: Option<i32>) -> Result<()> {
        match name {
            "list" => self.defining_list = Some(ListDefinition { depth: self.states.len(), id: 0, levels: Vec::new() }),
            "listlevel" => self.defining_level = Some(LevelDefinition { depth: self.states.len(), value: ListLevel::default() }),
            "levelnfc" | "levelnfcn" => { if let Some(level) = self.defining_level.as_mut() { level.value.format = arg.unwrap_or(0); } },
            "levelstartat" => {
                if let Some(level) = self.defining_level.as_mut() { level.value.start = arg.unwrap_or(1); }
                else if let Some(value) = self.defining_override.as_mut() { value.starts.insert(value.level, arg.unwrap_or(1)); }
            },
            "listid" => { if let Some(value) = self.defining_list.as_mut() { value.id = arg.unwrap_or(0); } if let Some(value) = self.defining_override.as_mut() { value.list = arg.unwrap_or(0); } },
            "listoverride" => self.defining_override = Some(Override { depth: self.states.len(), list: 0, number: 0, starts: HashMap::new(), level: 0, next_level: 0 }),
            "lfolevel" => { if let Some(value) = self.defining_override.as_mut() { if value.next_level > 8 { return Err(invalid()); } value.level = value.next_level; value.next_level += 1; } },
            "ls" => { if let Some(value) = self.defining_override.as_mut() { value.number = arg.unwrap_or(0); } },
            "leveltext" | "levelnumbers" => self.state_mut().destination = Destination::Skip,
            _ => {},
        }
        Ok(())
    }
}
fn blocks(values: Vec<Block>) -> Vec<Node> {
    let mut values = values.into_iter().peekable(); let mut result = Vec::new();
    while let Some(value) = values.peek() {
        if let Block::Listed { number, level, format, start, .. } = value { let data = (*number, *level, *format, *start); result.push(list(&mut values, data)); }
        else if let Some(Block::Plain(node)) = values.next() { result.push(node); }
    }
    result
}
fn list(values: &mut std::iter::Peekable<std::vec::IntoIter<Block>>, (number, level, format, start): (i32, usize, i32, i32)) -> Node {
    let mut items = Vec::new();
    while matches!(values.peek(), Some(Block::Listed { number: value, level: depth, format: style, .. }) if *value == number && *depth == level && *style == format) {
        let Some(Block::Listed { child, .. }) = values.next() else { break; }; let mut children = vec![child];
        while let Some(Block::Listed { number: nested, level: depth, format: style, start: ordinal, .. }) = values.peek() {
            if *depth <= level { break; } let data = (*nested, *depth, *style, *ordinal); children.push(list(values, data));
        }
        items.push(element(ElementKind::ListItem, Style::default(), children));
    }
    let list_style = match format { 1 => super::ListStyle::UpperRoman, 2 => super::ListStyle::LowerRoman,
        3 => super::ListStyle::UpperAlpha, 4 => super::ListStyle::LowerAlpha, 23 => super::ListStyle::Disc, _ => super::ListStyle::Decimal };
    element(if format == 23 { ElementKind::UnorderedList } else { ElementKind::OrderedList { start } }, Style { list_style: Some(list_style), ..Style::default() }, items)
}
fn charset(value: i32) -> Option<&'static Encoding> {
    match value { 128 => codepage(932), 129 => codepage(949), 134 => codepage(936), 136 => codepage(950), 161 => codepage(1253), 162 => codepage(1254), 163 => codepage(1258), 177 => codepage(1255), 178 => codepage(1256), 186 => codepage(1257), 204 => codepage(1251), 222 => codepage(874), 238 => codepage(1250), _ => None }
}
fn codepage(value: i32) -> Option<&'static Encoding> {
    match value {
        65001 => Some(encoding_rs::UTF_8), 10000 => Some(encoding_rs::MACINTOSH), 932 => Some(encoding_rs::SHIFT_JIS), 936 => Some(encoding_rs::GBK), 949 => Some(encoding_rs::EUC_KR), 950 => Some(encoding_rs::BIG5), 874 => Some(encoding_rs::WINDOWS_874),
        1250 => Some(encoding_rs::WINDOWS_1250), 1251 => Some(encoding_rs::WINDOWS_1251), 1252 => Some(WINDOWS_1252), 1253 => Some(encoding_rs::WINDOWS_1253), 1254 => Some(encoding_rs::WINDOWS_1254), 1255 => Some(encoding_rs::WINDOWS_1255), 1256 => Some(encoding_rs::WINDOWS_1256), 1257 => Some(encoding_rs::WINDOWS_1257), 1258 => Some(encoding_rs::WINDOWS_1258), 866 => Some(encoding_rs::IBM866), _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn as_element(node: &Node) -> &Element {
        let Node::Element(value) = node else { panic!("expected element"); }; value
    }
    fn cell_text(nodes: &[Node], text: &mut String) {
        for node in nodes { match node {
            Node::Text(value) => text.push_str(value),
            Node::Element(value) => cell_text(&value.children, text),
        } }
    }

    #[test]
    fn rtf_table_merges_preserve_spans_text_order_and_original_cell_style() {
        let source = concat!(
            r"{\rtf1{\colortbl;\red255\green0\blue0;}",
            r"\trowd\clmgf\clvmgf\clcbpat1\cellx100\clmrg\clvmgf\cellx200\cellx300\intbl\b A\cell\b0 B\cell C\cell\row",
            r"\trowd\clmgf\clvmrg\cellx100\clmrg\clvmrg\cellx200\cellx300\intbl D\cell E\cell F\cell\row",
            r"\trowd\clmgf\clvmrg\cellx100\clmrg\clvmrg\cellx200\cellx300\intbl G\cell H\cell I\cell\row}",
        );
        let document = parse(source).expect("merged RTF table");
        assert_eq!(document.nodes.len(), 1);
        let table = as_element(&document.nodes[0]);
        assert!(matches!(table.kind, ElementKind::Table));
        assert_eq!(table.children.len(), 3);
        let first_row = as_element(&table.children[0]);
        assert_eq!(first_row.children.len(), 2);
        let merged = as_element(&first_row.children[0]);
        assert!(matches!(merged.kind, ElementKind::Cell { columns: 2, rows: 3, .. }));
        assert!(matches!(merged.style.background, Some(CssColor::Rgba([255, 0, 0, 255]))));
        let mut text = String::new(); cell_text(&merged.children, &mut text);
        assert_eq!(text, "ABDEGH");
        let first_run = as_element(&as_element(&merged.children[0]).children[0]);
        assert!(matches!(first_run.style.weight, Some(super::super::Weight::Absolute(700.0))));
        for (index, expected) in ["C", "F", "I"].into_iter().enumerate() {
            let row = as_element(&table.children[index]);
            assert_eq!(row.children.len(), if index == 0 { 2 } else { 1 });
            let neighbor = as_element(row.children.last().expect("neighbor cell"));
            assert!(matches!(neighbor.kind, ElementKind::Cell { columns: 1, rows: 1, .. }));
            text.clear(); cell_text(&neighbor.children, &mut text); assert_eq!(text, expected);
        }
    }

    #[test]
    fn rtf_table_merges_retain_nested_tables_between_paragraphs() {
        let source = concat!(r"{\rtf1\trowd\clmgf\cellx100\clmrg\cellx200\cellx300\intbl Before",
            r"\itap2 Nested\nestcell\nestrow\itap1 After\cell Tail\cell Side\cell\row}");
        let document = parse(source).expect("nested merged RTF table");
        let table = as_element(&document.nodes[0]);
        let row = as_element(&table.children[0]);
        assert_eq!(row.children.len(), 2);
        let merged = as_element(&row.children[0]);
        assert!(matches!(merged.kind, ElementKind::Cell { columns: 2, rows: 1, .. }));
        assert_eq!(merged.children.len(), 4);
        assert!(matches!(as_element(&merged.children[1]).kind, ElementKind::Table));
        let mut text = String::new(); cell_text(&merged.children, &mut text);
        assert_eq!(text, "BeforeNestedAfterTail");
        text.clear(); cell_text(&as_element(&row.children[1]).children, &mut text);
        assert_eq!(text, "Side");
    }

    #[test]
    fn rtf_unicode_fallback_skips_complete_controls_without_side_effects() {
        for source in [
            r"{\rtf1\uc1\u65\tab B}",
            r"{\rtf1\uc1\u65\par B}",
            r"{\rtf1\uc1\u65\b B}",
            r"{\rtf1\uc1\u65\'e9B}",
            r"{\rtf1\uc1\u65\{B}",
            r"{\rtf1\uc1\u65\bin5 {\}xyB}",
            r"{\rtf1\uc2\u65\'81\'40B}",
            r"{\rtf1\uc2\u65?\tab B}",
        ] {
            let document = parse(source).expect("valid fallback controls");
            assert_eq!(&*document.text, "AB", "{source}");
            assert_eq!(document.nodes.len(), 1, "fallback must not emit a paragraph: {source}");
            let Node::Element(paragraph) = &document.nodes[0] else { panic!("paragraph"); };
            assert!(paragraph.children.iter().all(|node| !matches!(node,
                Node::Element(element) if element.style.weight.is_some())), "fallback must not apply bold: {source}");
        }
        let document = parse(r"{\rtf1\uc1\u65?\b B\tab C}").expect("controls after fallback");
        assert_eq!(&*document.text, "AB\tC");
        let Node::Element(paragraph) = &document.nodes[0] else { panic!("paragraph"); };
        assert!(paragraph.children.iter().any(|node| matches!(node,
            Node::Element(element) if element.style.weight.is_some())), "ordinary bold must still apply");
    }

    #[test]
    fn rtf_unicode_fallback_ends_at_groups_and_preserves_scoped_byte_counts() {
        for (source, expected) in [
            (r"{\rtf1\uc2\u20013{B}C}", "中BC"),
            (r"{\rtf1{\uc2\u20013}BC}", "中BC"),
            (r"{\rtf1\uc1{\uc2\u20013\'d6\'d0}\u25991?X}", "中文X"),
            ("{\\rtf1\\ansicpg65001\\uc3\\u20013中X}", "中X"),
            ("{\\rtf1\\uc2\\u20013\r\n??X}", "中X"),
            (r"{\rtf1\uc1\u-10179?\u-8704?X}", "😀X"),
        ] {
            assert_eq!(&*parse(source).expect("valid Unicode fallback").text, expected, "{source}");
        }
    }
}
