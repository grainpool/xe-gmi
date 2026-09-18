//! Minimal JSON value + writer, no parsing ever needed.
//!
//! Formatting is normative: 2-space indent, `": "` after keys, one element per line, numbers
//! preformatted with the same decimals as the text output, control chars escaped `\u00XX`.
//! Scalar arrays (numbers/strings/bools/nulls) render on one line; anything else nests.

use std::fmt::Write as _;

#[derive(Debug, Clone)]
pub enum Json {
    Null,
    Bool(bool),
    /// Preformatted number text (`12.40`, `5.0`, `34359738368`).
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    /// Key order is emission order (the schema's `properties` order).
    Obj(Vec<(String, Json)>),
}

pub fn write(j: &Json, out: &mut String) {
    write_at(j, out, 0);
}

/// One-line form for the streaming `events --json` format: identical separators, no newlines.
pub fn write_line(j: &Json, out: &mut String) {
    match j {
        Json::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_line(item, out);
            }
            out.push(']');
        }
        Json::Obj(fields) => {
            out.push('{');
            for (i, (k, v)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_str(k, out);
                out.push_str(": ");
                write_line(v, out);
            }
            out.push('}');
        }
        other => write_at(other, out, 0),
    }
}

pub fn num_f(v: f64, decimals: usize) -> Json {
    Json::Num(format!("{v:.decimals$}"))
}

pub fn num_u(v: u64) -> Json {
    Json::Num(v.to_string())
}

pub fn num_i(v: i64) -> Json {
    Json::Num(v.to_string())
}

fn write_at(j: &Json, out: &mut String, depth: usize) {
    match j {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => {
            let _ = write!(out, "{b}");
        }
        Json::Num(n) => out.push_str(n),
        Json::Str(s) => write_str(s, out),
        Json::Arr(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            if items
                .iter()
                .all(|i| matches!(i, Json::Num(_) | Json::Bool(_) | Json::Str(_) | Json::Null))
            {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    let mut one = String::new();
                    write_at(item, &mut one, 0);
                    out.push_str(&one);
                }
                out.push(']');
                return;
            }
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                indent(out, depth + 1);
                write_at(item, out, depth + 1);
            }
            out.push('\n');
            indent(out, depth);
            out.push(']');
        }
        Json::Obj(fields) => {
            if fields.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            for (i, (k, v)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                indent(out, depth + 1);
                write_str(k, out);
                out.push_str(": ");
                write_at(v, out, depth + 1);
            }
            out.push('\n');
            indent(out, depth);
            out.push('}');
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes() {
        let mut s = String::new();
        write(
            &Json::Obj(vec![
                ("b".into(), Json::Str("say \"hi\"".into())),
                ("a".into(), Json::Str("back\\slash".into())),
                ("ctrl".into(), Json::Str("line\nbreak\tand\u{1}\r".into())),
                (
                    "nums".into(),
                    Json::Arr(vec![num_f(12.40, 2), num_u(7), num_i(-3)]),
                ),
                ("empty".into(), Json::Arr(vec![])),
                ("nothing".into(), Json::Null),
            ]),
            &mut s,
        );
        let expected = "{\n  \"b\": \"say \\\"hi\\\"\",\n  \"a\": \"back\\\\slash\",\n  \"ctrl\": \"line\\nbreak\\tand\\u0001\\r\",\n  \"nums\": [12.40, 7, -3],\n  \"empty\": [],\n  \"nothing\": null\n}";
        assert_eq!(s, expected);
        // Emission order is insertion order, not key order.
        assert!(s.find("\"b\"").unwrap() < s.find("\"a\"").unwrap());
    }
}
