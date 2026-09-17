//! The prose and the type badges of documented values, and the
//! `Field | Type | Description` tables built from them.
//!
//! The type of a value is read from its schema, either a `schemars` document or
//! an OpenAPI component schema, rather than from its prose, so a value the schema
//! does not describe has no type to show and is left out of a [`doc_table`].

use maud::{html, Markup};
use serde_json::Value;

use crate::components::widget::html_table;
use crate::markdown::{collapse_ws, doc_blocks, inline, split_paragraph};
use crate::schema::{self, Defs};

/// The badge naming a value's type, carrying `extra_class` on top of its own
/// classes. `None` when the schema names no type.
pub fn type_badge<'a>(
    schema: &'a Value,
    defs: Defs<'a, '_>,
    extra_class: Option<&str>,
) -> Option<Markup> {
    let name = schema::type_name(schema, defs)?;
    Some(html! {
        span class={
            "badge setting-type " (schema::type_class(schema)) " " (extra_class.unwrap_or_default())
        } {
            (inline(&name))
        }
    })
}

/// Reports on stderr every value the schema describes but gives no type to,
/// which would leave its badge blank. `context` names the section they appear
/// in.
pub fn warn_untyped(context: &str, fields: &[String]) {
    for name in fields {
        eprintln!("warning: {context}: field `{name}` has no type in the schema");
    }
}

/// The prose documenting one value, a setting or a field of an object, as
/// the blocks its author wrote: paragraphs, lists, and the callouts
/// [`doc_blocks`] boxes. Any other heading is set as its own line of bold
/// text rather than as a section of the page.
pub fn value_markdown(doc: &str) -> Markup {
    doc_blocks(doc, &mut |_, inner| html! { p class="mb-0" { strong { (inner) } } })
}

/// [`value_markdown`], with `lead` set at the head of the prose's first
/// paragraph, which then carries `class`. Prose that opens with something other
/// than a paragraph, such as a rustdoc section rendered as a callout, takes
/// `lead` ahead of it instead, in a paragraph of its own only where `class`
/// names one.
pub fn value_markdown_with_lead(lead: Markup, class: Option<&str>, doc: &str) -> Markup {
    let Some((para, rest)) = split_paragraph(doc) else {
        return html! {
            @match class {
                Some(class) => { p class=(class) { (lead) } }
                None => { (lead) }
            }
            (value_markdown(doc))
        };
    };
    html! {
        p class=[class] { (lead) (inline(&collapse_ws(para))) }
        (value_markdown(rest))
    }
}

/// The prose of one field: its description, and a short note set under the
/// field name, such as `(optional)` or how the key is switched off.
pub struct FieldDoc {
    pub doc: Option<String>,
    pub note: Option<String>,
}

impl From<Option<String>> for FieldDoc {
    fn from(doc: Option<String>) -> Self {
        Self { doc, note: None }
    }
}

/// A `Field | Type | Description` table over the properties of an object
/// schema, with each field's prose taken from `doc`, alongside the names of the
/// fields the schema gives no type to.
pub fn doc_table<'a, D: Into<FieldDoc>>(
    schema: &'a Value,
    defs: Defs<'a, '_>,
    doc: impl Fn(&str) -> D,
) -> (Markup, Vec<String>) {
    let fields = schema::properties(schema)
        .into_iter()
        .map(|(name, prop)| (name.to_string(), prop, doc(name).into()));
    field_table(fields, defs)
}

/// A `Field | Type | Description` table over fields the caller names itself,
/// each given as its name, the schema its type badge is read from, and its
/// prose, alongside the names of the fields the schema gives no type to.
pub fn field_table<'a>(
    fields: impl IntoIterator<Item = (String, &'a Value, FieldDoc)>,
    defs: Defs<'a, '_>,
) -> (Markup, Vec<String>) {
    let mut untyped: Vec<String> = Vec::new();
    let rows: Vec<Vec<Markup>> = fields
        .into_iter()
        .map(|(name, schema, field)| {
            let badge = type_badge(schema, defs, None);
            if badge.is_none() {
                untyped.push(name.clone());
            }
            let name = html! {
                code class="doc-field" { (name) }
                @if let Some(note) = &field.note {
                    br;
                    small class="doc-field-note" { (inline(note)) }
                }
            };
            let doc = value_markdown(field.doc.as_deref().unwrap_or_default());
            vec![name, badge.unwrap_or_default(), doc]
        })
        .collect();
    (html_table(&["Field", "Type", "Description"], &rows), untyped)
}
