//! Only retained declarations with var() need deferred computation. Custom
//! declarations themselves are removed by the reference preview's whitelist.
use std::{borrow::Cow, sync::Arc};
use cssparser::{ParseError, Parser, ParserInput, ToCss, Token};
use super::{Property, Style, typed_value};

const MAX_DEPTH: usize = 16;
const MAX_EXPANSION: usize = 800_000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeferredStyles(Vec<(Property, Arc<str>)>);
impl DeferredStyles {
    pub(super) fn push(&mut self, property: Property, source: Arc<str>) { self.0.push((property, source)); }
    pub(super) fn remove(&mut self, property: Property) { self.0.retain(|(key, _)| *key != property); }
    pub(super) fn is_empty(&self) -> bool { self.0.is_empty() }
    pub(crate) fn weight(&self) -> u64 {
        (std::mem::size_of::<Self>() + self.0.capacity() * std::mem::size_of::<(Property, Arc<str>)>()) as u64
            + self.0.iter().map(|(_, source)| source.len() as u64).sum::<u64>()
    }
}

impl Style {
    /// Lookup supplies computed ancestor custom properties, with case-sensitive
    /// names. The document never retains a UI theme or a second styled tree.
    pub fn resolve_variables<'a, 'v>(&'a self, lookup: impl Fn(&str) -> Option<&'v str>) -> Cow<'a, Style> {
        let Some(values) = &self.variable_styles else { return Cow::Borrowed(self); };
        let mut style = self.clone();
        style.variable_styles = None;
        let mut expanded = String::new();
        let mut normalized = String::new();
        for (property, source) in &values.0 {
            expanded.clear(); normalized.clear();
            let mut input = ParserInput::new(source);
            let mut input = Parser::new(&mut input);
            if expand(&mut input, &lookup, &mut expanded, 0).is_err() { continue; }
            let mut input = ParserInput::new(&expanded);
            let mut input = Parser::new(&mut input);
            let mut limited = false;
            if let Ok(value) = typed_value(*property, &mut input, &mut normalized, &mut limited)
                && input.expect_exhausted().is_ok() && !limited
            {
                style.resets.set(*property, None);
                value.apply(&mut style);
            }
            // Invalid at computed-value time keeps the unset reset installed
            // by Pending. It cannot revive a preceding authored declaration.
        }
        Cow::Owned(style)
    }
}

pub(super) fn pending(input: &mut Parser<'_, '_>, limited: &mut bool) -> Option<Arc<str>> {
    let start = input.position();
    let mut found = false;
    let mut exceeded = false;
    let result = validate(input, &mut found, &mut exceeded, 0, true);
    *limited |= found && exceeded;
    result.ok()?;
    found.then(|| Arc::from(input.slice_from(start)))
}

fn validate<'i>(input: &mut Parser<'i, '_>, found: &mut bool, limited: &mut bool, depth: usize, declaration: bool) -> Result<(), ParseError<'i, ()>> {
    if depth > MAX_DEPTH { *limited = true; return Err(input.new_custom_error(())); }
    while !input.is_exhausted() {
        match input.next_including_whitespace_and_comments()?.clone() {
            Token::Function(name) => {
                input.parse_nested_block(|input| {
                    if name.eq_ignore_ascii_case("var") {
                        *found = true;
                        let name = input.expect_ident()?;
                        if !name.starts_with("--") || name.len() == 2 { return Err(input.new_custom_error(())); }
                        if input.is_exhausted() { return Ok(()); }
                        input.expect_comma()?;
                    }
                    validate(input, found, limited, depth + 1, name.eq_ignore_ascii_case("var"))
                })?;
            },
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                input.parse_nested_block(|input| validate(input, found, limited, depth + 1, false))?;
            },
            token if token.is_parse_error() || declaration && matches!(token, Token::Delim('!') | Token::Semicolon) => return Err(input.new_custom_error(())),
            _ => {},
        }
    }
    Ok(())
}

fn expand<'i, 'v>(input: &mut Parser<'i, '_>, lookup: &impl Fn(&str) -> Option<&'v str>, output: &mut String, depth: usize) -> Result<(), ParseError<'i, ()>> {
    if depth > MAX_DEPTH { return Err(input.new_custom_error(())); }
    while !input.is_exhausted() {
        let token = input.next_including_whitespace_and_comments()?.clone();
        match &token {
            Token::Function(name) if name.eq_ignore_ascii_case("var") => input.parse_nested_block(|input| {
                let name = input.expect_ident_cloned()?;
                let fallback = !input.is_exhausted();
                if fallback { input.expect_comma()?; }
                // Comments separate replacement tokens without manufacturing
                // whitespace for calc() operators or joining a number and unit.
                output.push_str("/**/");
                if let Some(value) = lookup(&name) {
                    if output.len().saturating_add(value.len()) > MAX_EXPANSION { return Err(input.new_custom_error(())); }
                    output.push_str(value);
                    input.expect_no_error_token()?;
                } else if fallback { expand(input, lookup, output, depth + 1)?; }
                else { return Err(input.new_custom_error(())); }
                output.push_str("/**/");
                Ok(())
            })?,
            Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                token.to_css(output).expect("writing CSS into a String cannot fail");
                input.parse_nested_block(|input| expand(input, lookup, output, depth + 1))?;
                output.push(match token { Token::SquareBracketBlock => ']', Token::CurlyBracketBlock => '}', _ => ')' });
            },
            _ => token.to_css(output).expect("writing CSS into a String cannot fail"),
        }
        if output.len() > MAX_EXPANSION { return Err(input.new_custom_error(())); }
    }
    Ok(())
}
