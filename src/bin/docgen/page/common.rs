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
use crate::markdown::{escape, inline, markdown};
use crate::schema::{self, Defs};

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

/// The prose of one field: its description, and a note set under the field
/// name for what the key does beyond carrying a value.
#[derive(Default)]
pub struct FieldDoc {
    pub doc: String,
    pub note: Option<String>,
}

impl From<String> for FieldDoc {
    fn from(doc: String) -> Self {
        Self { doc, note: None }
    }
}

/// A `Field | Type | Description` table over the properties of an object
/// schema, with each field's prose taken from `doc`. `context` names the
/// section for [`warn_untyped`].
pub fn doc_table<'a, D: Into<FieldDoc>>(
    context: &str,
    schema: &'a Value,
    defs: Defs<'a, '_>,
    doc: impl Fn(&str) -> D,
) -> String {
    let rows: Vec<Vec<String>> = schema::properties(schema)
        .into_iter()
        .map(|(name, prop)| {
            let badge = type_badge(prop, defs);
            if badge.is_empty() {
                warn_untyped(context, name);
            }
            let field = doc(name).into();
            let name = match &field.note {
                Some(note) => format!(
                    "<code class=\"doc-field\">{}</code>\
                     <br><small class=\"doc-field-note\">{}</small>",
                    escape(name),
                    inline(note)
                ),
                None => {
                    format!("<code class=\"doc-field\">{}</code>", escape(name))
                }
            };
            // Block markdown, so a field's doc keeps the paragraphs and lists
            // its author wrote. A heading has no place in a table cell, so it
            // is set as its own line of bold text instead.
            let desc = markdown(&field.doc, &mut |_, inner| {
                format!("<p class=\"mb-0\"><strong>{inner}</strong></p>\n")
            });
            vec![name, badge, desc]
        })
        .collect();
    html_table(&["Field", "Type", "Description"], &rows)
}
