use std::sync::Arc;
use html5ever::{parse_document, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use crate::{Error, Result};
use super::{Direction, Document, Element, ElementKind, Node, css::style};
use super::super::text_whitespace;

mod data_url;

pub(crate) fn parse(source: &str) -> Result<Document> {
    if !super::source_within_limit(source) { return Err(Error::ResourceLimit("富文本超过排版预览上限".into())); }
    let dom = parse_document(RcDom::default(), Default::default()).one(source);
    let mut nodes = Vec::new(); let mut visited = 0;
    walk(&dom.document, &mut nodes, 0, &mut visited)?;
    Document::new(nodes)
}
fn walk(handle: &Handle, output: &mut Vec<Node>, depth: usize, visited: &mut usize) -> Result<()> {
    *visited += 1;
    if depth > 96 || *visited > 20_000 { return Err(Error::ResourceLimit("富文本结构过于复杂".into())); }
    match &handle.data {
        NodeData::Text { contents } => { let text = contents.borrow(); if !text.is_empty() { output.push(Node::Text(text.to_string().into())); } },
        NodeData::Element { name, attrs, .. } => {
            let tag = name.local.as_ref();
            if matches!(tag, "script" | "style" | "head" | "title" | "template" | "iframe" | "object" | "embed" | "noscript" | "svg" | "math" | "noembed" | "noframes" | "xmp") { return Ok(()); }
            let attrs = attrs.borrow();
            // DOMPurify trims every attribute used by the reference preview
            // before its whitelist and provider-specific normalization.
            let attribute = |key: &str| attrs.iter().find(|attribute| attribute.name.ns.is_empty() && attribute.name.local.as_ref() == key)
                .map(|attribute| attribute.value.trim_matches(text_whitespace));
            if tag == "img" {
                let alt = attribute("alt").filter(|text| !text.is_empty()).unwrap_or("");
                let source = attribute("src").unwrap_or("");
                if let Some(value) = data_url::body(source) {
                    // Invalid embedded images remain visible failures; never fetch a URL.
                    let data = data_url::decode(value).map(crate::images::EncodedImageInput::new);
                    output.push(Node::Element(Element { kind: ElementKind::Image { data, alt: alt.into() }, style: style(attribute("style").unwrap_or(""))?, direction: direction(attribute("dir")), title: attribute("title").map(Arc::from), children: Arc::default(), first_strong: None }));
                } else { output.push(Node::Text(format!("[{}]", if alt.is_empty() { "外部图片" } else { alt }).into())); }
                return Ok(());
            }
            let kind = match tag {
                "p" => Some(ElementKind::Paragraph), "div" => Some(ElementKind::Container), "span" => Some(ElementKind::Span),
                "b" | "strong" => Some(ElementKind::Bold), "i" | "em" => Some(ElementKind::Italic), "u" => Some(ElementKind::Underline),
                "s" | "del" => Some(ElementKind::Strike), "mark" => Some(ElementKind::Mark), "small" => Some(ElementKind::Small),
                "sub" => Some(ElementKind::Subscript), "sup" => Some(ElementKind::Superscript), "br" => Some(ElementKind::Break),
                "pre" => Some(ElementKind::Pre), "code" => Some(ElementKind::Code), "blockquote" => Some(ElementKind::Quote),
                "ul" => Some(ElementKind::UnorderedList), "ol" => Some(ElementKind::OrderedList { start: attribute("start").and_then(list_start).unwrap_or(1) }), "li" => Some(ElementKind::ListItem),
                "table" => Some(ElementKind::Table), "thead" => Some(ElementKind::TableHead), "tbody" => Some(ElementKind::TableBody), "tfoot" => Some(ElementKind::TableFoot), "tr" => Some(ElementKind::TableRow),
                "td" | "th" => Some(ElementKind::Cell { heading: tag == "th", columns: span(attribute("colspan")), rows: span(attribute("rowspan")) }),
                "caption" => Some(ElementKind::Caption), "hr" => Some(ElementKind::Rule), "a" => Some(ElementKind::Link),
                "h1" => Some(ElementKind::Heading(1)), "h2" => Some(ElementKind::Heading(2)), "h3" => Some(ElementKind::Heading(3)), "h4" => Some(ElementKind::Heading(4)), "h5" => Some(ElementKind::Heading(5)), "h6" => Some(ElementKind::Heading(6)), _ => None,
            };
            let mut children = Vec::new(); for child in handle.children.borrow().iter() { walk(child, &mut children, depth + 1, visited)?; }
            if let Some(kind) = kind { output.push(Node::Element(Element { kind, style: style(attribute("style").unwrap_or(""))?, direction: direction(attribute("dir")), title: attribute("title").map(Arc::from), children: children.into(), first_strong: None })); }
            else { output.extend(children); }
        },
        NodeData::Document => { for child in handle.children.borrow().iter() { walk(child, output, depth + 1, visited)?; } },
        _ => {},
    }
    Ok(())
}
fn list_start(value: &str) -> Option<i32> {
    // HTML integer parsing accepts a signed decimal prefix, not a complete
    // Rust integer or JS Number. OL's reflected start is a signed 32-bit long.
    // https://html.spec.whatwg.org/multipage/common-microsyntaxes.html#rules-for-parsing-integers
    let value = value.trim_start_matches(|character| matches!(character, '\t' | '\n' | '\x0c' | '\r' | ' ')).as_bytes();
    let (negative, digits) = match value.first() {
        Some(b'-') => (true, &value[1..]), Some(b'+') => (false, &value[1..]), _ => (false, value),
    };
    if !digits.first().is_some_and(u8::is_ascii_digit) { return None; }
    let mut number = 0i32;
    for &digit in digits.iter().take_while(|digit| digit.is_ascii_digit()) {
        number = number.checked_mul(10)?;
        let digit = i32::from(digit - b'0');
        number = if negative { number.checked_sub(digit)? } else { number.checked_add(digit)? };
    }
    Some(number)
}

fn span(value: Option<&str>) -> u8 {
    // The reference first applies Number(), then clamps to [1, 20]. Parsing
    // HTML integers here would disagree for exponents and radix prefixes.
    // https://tc39.es/ecma262/multipage/abstract-operations.html#sec-stringtonumber
    let value = value.unwrap_or("");
    let radix = match value.as_bytes() {
        [b'0', b'x' | b'X', ..] => Some(16u8),
        [b'0', b'o' | b'O', ..] => Some(8u8),
        [b'0', b'b' | b'B', ..] => Some(2u8),
        _ => None,
    };
    if let Some(radix) = radix {
        let digits = &value.as_bytes()[2..];
        if digits.is_empty() { return 1; }
        let mut number = 0u8;
        for &byte in digits {
            let digit = match byte {
                b'0'..=b'9' => byte - b'0', b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10, _ => return 1,
            };
            if digit >= radix { return 1; }
            // Saturate only the result, continuing to validate every digit.
            number = number.saturating_mul(radix).saturating_add(digit).min(20);
        }
        return number.max(1);
    }
    let number = match value {
        "Infinity" | "+Infinity" => 20.0,
        "-Infinity" => 1.0,
        value if value.bytes().all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'+' | b'-' | b'e' | b'E')) => {
            value.parse::<f64>().unwrap_or(1.0)
        },
        _ => 1.0,
    };
    number.clamp(1.0, 20.0) as u8
}

fn direction(value: Option<&str>) -> Option<Direction> {
    // Compare the whole keyword after the shared sanitizer normalization.
    let value = value?;
    if value.eq_ignore_ascii_case("ltr") { Some(Direction::Ltr) }
    else if value.eq_ignore_ascii_case("rtl") { Some(Direction::Rtl) }
    else if value.eq_ignore_ascii_case("auto") { Some(Direction::Auto) }
    else { None }
}
