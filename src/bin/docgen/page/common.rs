//! Shared by the pages generated from the crate's own configuration types.
//!
//! The type of every documented value is read from the `schemars` schema of the
//! type it belongs to, so it is never written down a second time in a doc
//! comment. That also decides what is documented at all: a field the schema
//! does not describe — one marked `#[schemars(skip)]`, because it is not part of
//! the file format — has no type to show and is left out. User-facing prose
//! lives in the doc comments.

use serde_json::Value;

use crate::html::html_table;
use crate::markdown::{escape, inline};
use crate::schema::{self, Defs};

/// Collapses the newlines from doc comments wrapped across lines into spaces.
pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A heading that is nothing but the verbatim name of a thing — a TOML table, a
/// test kind. Marked so `docs.css` can set it apart from a heading that merely
/// mentions one ("Root defaults (`[default]`)"), which CSS cannot tell apart on
/// its own: `:first-child` ignores the text around the name.
pub fn name_heading(name: &str) -> String {
    format!("<code class=\"doc-name\">{}</code>", escape(name))
}

/// The type of a value, as the badge a documented setting or table row carries.
/// Empty when the schema names no type, which is reported by the caller rather
/// than rendered as an empty pill.
pub fn type_badge<'a>(schema: &'a Value, defs: Defs<'a, '_>) -> String {
    let name = schema::type_name(schema, defs);
    if name.is_empty() {
        return String::new();
    }
    format!(
        "<span class=\"badge setting-type {}\">{}</span>",
        schema::type_class(schema),
        inline(&name)
    )
}

/// Reports a value the schema describes but gives no type to, which would leave
/// its badge blank. `context` names the section it appears in.
pub fn warn_untyped(context: &str, name: &str) {
    eprintln!("warning: {context}: field `{name}` has no type in the schema");
}

/// A `Field | Type | Description` table over the properties of an object
/// schema, with each field's prose taken from `doc`. `context` names the
/// section for [`warn_untyped`].
pub fn doc_table<'a>(
    context: &str,
    schema: &'a Value,
    defs: Defs<'a, '_>,
    doc: impl Fn(&str) -> String,
) -> String {
    let rows: Vec<Vec<String>> = schema::properties(schema)
        .into_iter()
        .map(|(name, prop)| {
            let badge = type_badge(prop, defs);
            if badge.is_empty() {
                warn_untyped(context, name);
            }
            vec![
                format!("<code>{}</code>", escape(name)),
                badge,
                inline(&doc(name)),
            ]
        })
        .collect();
    html_table(&["Field", "Type", "Description"], &rows)
}
