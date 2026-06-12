//! Parsing of `///` doc-comment annotations.
//!
//! Wisp shaders carry their metadata in WGSL doc comments: free description text
//! interleaved with `@name`/`@name(args)` annotations, e.g.
//!
//! ```wgsl
//! /// Overall strength of the effect.
//! /// @min(0.0) @max(1.0) @default(0.5)
//! level: f32,
//! ```
//!
//! This module is pure string processing; which annotations are meaningful where
//! is decided by [`crate::schema`].

use thiserror::Error;

/// A single `@name` or `@name(args)` annotation.
#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    pub name: String,
    pub args: Vec<Arg>,
}

/// One argument within an annotation's parentheses.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    /// A `key = value` pair, e.g. `target = "buffer_a"`.
    Named(String, Value),
    /// A bare value, e.g. the `0.5` in `@default(0.5)` or the flag `persistent`.
    Pos(Value),
}

/// An annotation argument value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A bare word, e.g. `persistent`.
    Ident(String),
    /// A numeric literal, e.g. `0.5` or `-3`.
    Number(f64),
    /// A double-quoted string, e.g. `"buffer_a"`.
    Str(String),
}

/// A parsed doc-comment block: free description text plus annotations.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Docs {
    pub description: String,
    pub annotations: Vec<Annotation>,
}

/// Errors produced when a doc comment's annotations are malformed.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum AnnotError {
    #[error("invalid argument `{arg}` in `@{annotation}(..)`")]
    BadArg { annotation: String, arg: String },
    #[error("missing closing `)` in `@{annotation}(..)`")]
    UnterminatedArgs { annotation: String },
    #[error("unterminated string in `@{annotation}(..)`")]
    UnterminatedString { annotation: String },
}

impl Annotation {
    /// The value of the `key = value` argument with the given key.
    pub fn named(&self, key: &str) -> Option<&Value> {
        self.args.iter().find_map(|arg| match arg {
            Arg::Named(k, v) if k == key => Some(v),
            _ => None,
        })
    }

    /// Whether the bare flag with the given name is present, e.g. `persistent`.
    pub fn flag(&self, name: &str) -> bool {
        self.args
            .iter()
            .any(|arg| matches!(arg, Arg::Pos(Value::Ident(s)) if s == name))
    }

    /// All positional arguments as numbers, if every argument is a positional number.
    pub fn pos_numbers(&self) -> Option<Vec<f64>> {
        self.args
            .iter()
            .map(|arg| match arg {
                Arg::Pos(Value::Number(n)) => Some(*n),
                _ => None,
            })
            .collect()
    }

    /// All positional arguments as strings, if every argument is a positional string.
    pub fn pos_strings(&self) -> Option<Vec<String>> {
        self.args
            .iter()
            .map(|arg| match arg {
                Arg::Pos(Value::Str(s)) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    /// The sole positional number, if the arguments are exactly that.
    pub fn single_number(&self) -> Option<f64> {
        match self.args.as_slice() {
            [Arg::Pos(Value::Number(n))] => Some(*n),
            _ => None,
        }
    }

    /// The sole positional string, if the arguments are exactly that.
    pub fn single_string(&self) -> Option<String> {
        match self.args.as_slice() {
            [Arg::Pos(Value::Str(s))] => Some(s.clone()),
            _ => None,
        }
    }
}

impl Value {
    /// The content of a `Str` or the word of an `Ident`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::Ident(s) => Some(s),
            Value::Number(_) => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
}

impl Docs {
    /// The first annotation with the given name.
    pub fn get(&self, name: &str) -> Option<&Annotation> {
        self.annotations.iter().find(|a| a.name == name)
    }

    /// Whether an annotation with the given name is present.
    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
}

/// Strip comment markers from raw doc comments and split them into plain text lines.
///
/// `naga` provides doc comments verbatim, including the `///`/`//!` or `/** .. */`
/// markers.
pub fn clean_lines(raw: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    for comment in raw {
        let comment = comment.trim();
        if let Some(block) = comment
            .strip_prefix("/**")
            .or_else(|| comment.strip_prefix("/*!"))
        {
            let block = block.strip_suffix("*/").unwrap_or(block);
            let clean = |l: &str| l.trim().trim_start_matches('*').trim().to_string();
            lines.extend(block.lines().map(clean));
        } else {
            let line = comment
                .strip_prefix("///")
                .or_else(|| comment.strip_prefix("//!"))
                .unwrap_or(comment);
            lines.push(line.trim().to_string());
        }
    }
    lines
}

/// Join raw doc comments into plain description text, without annotation parsing.
pub fn clean_text(raw: &[String]) -> String {
    normalize_ws(&clean_lines(raw).join(" "))
}

/// Parse raw doc comments into description text and annotations.
pub fn parse_docs(raw: &[String]) -> Result<Docs, AnnotError> {
    let mut description = String::new();
    let mut annotations = Vec::new();
    for line in clean_lines(raw) {
        parse_line(&line, &mut description, &mut annotations)?;
        description.push(' ');
    }
    let description = normalize_ws(&description);
    Ok(Docs {
        description,
        annotations,
    })
}

/// Parse one cleaned line, accumulating description text and annotations.
fn parse_line(
    line: &str,
    text: &mut String,
    annotations: &mut Vec<Annotation>,
) -> Result<(), AnnotError> {
    let mut i = 0;
    while let Some(off) = line[i..].find('@') {
        let at = i + off;
        // An annotation `@` must sit at a word boundary and be followed by an ident.
        let prev_ok = line[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        let name_len = ident_len(&line[at + 1..]);
        if !prev_ok || name_len == 0 {
            text.push_str(&line[i..at + 1]);
            i = at + 1;
            continue;
        }
        text.push_str(&line[i..at]);
        let name = line[at + 1..at + 1 + name_len].to_string();
        let mut end = at + 1 + name_len;
        let mut args = Vec::new();
        if line[end..].starts_with('(') {
            let close = find_close(&line[end..]).map_err(|err| match err {
                Unclosed::Args => AnnotError::UnterminatedArgs {
                    annotation: name.clone(),
                },
                Unclosed::Str => AnnotError::UnterminatedString {
                    annotation: name.clone(),
                },
            })?;
            args = parse_args(&line[end + 1..end + close], &name)?;
            end += close + 1;
        }
        annotations.push(Annotation { name, args });
        i = end;
    }
    text.push_str(&line[i..]);
    Ok(())
}

/// The byte length of the leading identifier in `s`, or 0 if there is none.
fn ident_len(s: &str) -> usize {
    let mut len = 0;
    for c in s.chars() {
        let ok = if len == 0 {
            c.is_ascii_alphabetic() || c == '_'
        } else {
            c.is_ascii_alphanumeric() || c == '_'
        };
        if !ok {
            break;
        }
        len += 1;
    }
    len
}

enum Unclosed {
    Args,
    Str,
}

/// Given `s` starting with `(`, the byte index of the matching `)`.
fn find_close(s: &str) -> Result<usize, Unclosed> {
    let mut depth = 0usize;
    let mut in_str = false;
    for (idx, c) in s.char_indices() {
        if in_str {
            if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(idx);
                }
            }
            _ => {}
        }
    }
    Err(if in_str {
        Unclosed::Str
    } else {
        Unclosed::Args
    })
}

fn parse_args(s: &str, annotation: &str) -> Result<Vec<Arg>, AnnotError> {
    let mut args = Vec::new();
    for piece in split_top_level(s, ',') {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        args.push(parse_arg(piece, annotation)?);
    }
    Ok(args)
}

/// Split on `sep` at paren depth zero, outside strings.
fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut in_str = false;
    for (idx, c) in s.char_indices() {
        if in_str {
            if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if c == sep && depth == 0 => {
                pieces.push(&s[start..idx]);
                start = idx + c.len_utf8();
            }
            _ => {}
        }
    }
    pieces.push(&s[start..]);
    pieces
}

fn parse_arg(s: &str, annotation: &str) -> Result<Arg, AnnotError> {
    match split_top_level(s, '=').as_slice() {
        [_] => Ok(Arg::Pos(parse_value(s, annotation)?)),
        [key, value] if is_ident(key.trim()) => Ok(Arg::Named(
            key.trim().to_string(),
            parse_value(value.trim(), annotation)?,
        )),
        _ => Err(AnnotError::BadArg {
            annotation: annotation.to_string(),
            arg: s.to_string(),
        }),
    }
}

fn parse_value(s: &str, annotation: &str) -> Result<Value, AnnotError> {
    let bad = || AnnotError::BadArg {
        annotation: annotation.to_string(),
        arg: s.to_string(),
    };
    if let Some(inner) = s.strip_prefix('"') {
        let inner = inner
            .strip_suffix('"')
            .ok_or_else(|| AnnotError::UnterminatedString {
                annotation: annotation.to_string(),
            })?;
        if inner.contains('"') {
            return Err(bad());
        }
        return Ok(Value::Str(inner.to_string()));
    }
    if let Ok(n) = s.parse::<f64>() {
        return Ok(Value::Number(n));
    }
    if is_ident(s) {
        return Ok(Value::Ident(s.to_string()));
    }
    Err(bad())
}

fn is_ident(s: &str) -> bool {
    !s.is_empty() && ident_len(s) == s.len()
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docs(lines: &[&str]) -> Docs {
        let raw: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        parse_docs(&raw).unwrap()
    }

    #[test]
    fn description_only() {
        let d = docs(&["/// Overall strength", "///   of the effect.  "]);
        assert_eq!(d.description, "Overall strength of the effect.");
        assert!(d.annotations.is_empty());
    }

    #[test]
    fn numbers_and_flags() {
        let d = docs(&["/// @min(0.0) @max(1.0) @default(0.5) @color"]);
        assert_eq!(d.get("min").unwrap().single_number(), Some(0.0));
        assert_eq!(d.get("max").unwrap().single_number(), Some(1.0));
        assert_eq!(d.get("default").unwrap().single_number(), Some(0.5));
        assert!(d.get("color").unwrap().args.is_empty());
    }

    #[test]
    fn negative_and_multi_numbers() {
        let d = docs(&["/// @default(-1.0, 0.25, 2, 1e3)"]);
        let nums = d.get("default").unwrap().pos_numbers().unwrap();
        assert_eq!(nums, vec![-1.0, 0.25, 2.0, 1000.0]);
    }

    #[test]
    fn named_args_flags_and_strings() {
        let d = docs(&[r#"/// @pass(target = "buffer_a", persistent, float, width = "$WIDTH/2")"#]);
        let pass = d.get("pass").unwrap();
        assert_eq!(pass.named("target").unwrap().as_str(), Some("buffer_a"));
        assert!(pass.flag("persistent"));
        assert!(pass.flag("float"));
        assert!(!pass.flag("nope"));
        assert_eq!(pass.named("width").unwrap().as_str(), Some("$WIDTH/2"));
    }

    #[test]
    fn description_interleaved_with_annotations() {
        let d = docs(&[
            "/// Feedback amount. @min(0.0)",
            "/// Try cranking it. @max(2.0)",
        ]);
        assert_eq!(d.description, "Feedback amount. Try cranking it.");
        assert_eq!(d.annotations.len(), 2);
    }

    #[test]
    fn email_like_text_is_not_an_annotation() {
        let d = docs(&["/// Contact mail@example.com for details"]);
        assert!(d.annotations.is_empty());
        assert_eq!(d.description, "Contact mail@example.com for details");
    }

    #[test]
    fn string_with_comma_and_parens() {
        let d = docs(&[r#"/// @labels("low, slow", "high (fast)")"#]);
        let labels = d.get("labels").unwrap().pos_strings().unwrap();
        assert_eq!(labels, vec!["low, slow", "high (fast)"]);
    }

    #[test]
    fn block_doc_comment() {
        let d = docs(&["/** Speed of the thing.\n * @min(0.0)\n */"]);
        assert_eq!(d.description, "Speed of the thing.");
        assert_eq!(d.get("min").unwrap().single_number(), Some(0.0));
    }

    #[test]
    fn unterminated_args() {
        let raw = vec!["/// @pass(target = \"a\"".to_string()];
        assert_eq!(
            parse_docs(&raw),
            Err(AnnotError::UnterminatedArgs {
                annotation: "pass".into()
            })
        );
    }

    #[test]
    fn unterminated_string() {
        let raw = vec!["/// @label(\"oops)".to_string()];
        assert_eq!(
            parse_docs(&raw),
            Err(AnnotError::UnterminatedString {
                annotation: "label".into()
            })
        );
    }

    #[test]
    fn bad_arg() {
        let raw = vec!["/// @min(0.0.0)".to_string()];
        assert!(matches!(parse_docs(&raw), Err(AnnotError::BadArg { .. })));
    }
}
