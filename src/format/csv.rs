//! `query` CSV: `, ` separator, `[unit]` header suffixes, quoting rules.

use crate::avail::Avail;
use crate::format::fields::{self, FieldDef, View};
use crate::format::NA;

/// A query field as requested by the user, with its resolved definition.
pub struct ParsedField {
    pub requested: String,
    pub def: &'static FieldDef,
    pub gt: Option<u32>,
}

/// Parse `--fields` values; the first unknown name is returned for the exit-2 message.
pub fn parse_fields(names: &[String]) -> Result<Vec<ParsedField>, String> {
    let mut out = Vec::with_capacity(names.len());
    for n in names {
        match fields::parse_name(n) {
            Some((def, gt)) => out.push(ParsedField {
                requested: n.clone(),
                def,
                gt,
            }),
            None => return Err(n.clone()),
        }
    }
    Ok(out)
}

/// Quote when the value contains `,`, `"` or a newline; inner quotes double.
pub fn quote(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Header line: the requested field names, `[unit]` appended when the field has a unit and
/// `--no-units` is not given.
pub fn header(fields: &[ParsedField], no_units: bool) -> String {
    fields
        .iter()
        .map(|f| {
            if !no_units && !f.def.unit.is_empty() {
                format!("{} [{}]", f.requested, f.def.unit)
            } else {
                f.requested.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// One CSV row for one device.
pub fn row(fields: &[ParsedField], view: &View) -> String {
    fields
        .iter()
        .map(|f| match fields::resolve(f.def.name, f.gt, view) {
            Avail::Value(c) => c.text,
            Avail::NotAvailable(_) => NA.to_string(),
        })
        .map(|t| quote(&t))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn quote_rules() {
        assert_eq!(quote("a,b"), "\"a,b\"");
        assert_eq!(quote("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(quote("plain"), "plain");
        assert_eq!(quote("line\nbreak"), "\"line\nbreak\"");
    }
}
