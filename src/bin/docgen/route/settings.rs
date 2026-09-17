//! The settings reference. `confique`'s `Meta` yields the nesting structure and
//! the environment-variable name alongside each field. The `schemars` schema of
//! the same types yields what `Meta` does not, being the type of each value and
//! the element type of a collection.

use std::collections::{BTreeMap, BTreeSet};

use confique::meta::{FieldKind, Meta};
use confique::Config as _;
use maud::{html, Markup};
use schemars::schema_for;
use serde_json::Value;

use id2202_autograder::config::settings::Settings;

use crate::components::{
    code_block, details, doc_table, slug, type_badge, value_markdown_with_lead, warn_untyped, Body,
};
use crate::schema::{self, Defs};

/// The repository's example settings file, embedded at build time.
const EXAMPLE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/example/settings.toml"));

/// Where a doc comment's `[TypeName]` link resolves to, by type name: the
/// markdown the link shows, and the id of the section it points at.
type Links = BTreeMap<String, (String, String)>;

/// The TOML table a nested settings struct is configured under.
fn table_name(prefix: &str) -> String {
    format!("[{}]", prefix.trim_end_matches('.'))
}

/// Rewrites the rustdoc intra-doc links in a doc comment (`[ServerSettings]`)
/// into markdown links to the section documenting that type. A name that is not
/// a documented type is left exactly as it was.
fn resolve(doc: &str, links: &Links) -> String {
    let mut out = String::new();
    let mut rest = doc;
    while let Some(at) = rest.find('[') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let Some(end) = after.find(']') else {
            break;
        };
        let (name, tail) = (after[..end].trim_matches('`'), &after[end + 1..]);
        match links.get(name) {
            // A `[text](url)` already carries its own target, so only a bare
            // `[Type]` is given one.
            Some((label, anchor)) if !tail.starts_with('(') => {
                out.push_str(&format!("[{label}](#{anchor})"));
            }
            _ => out.push_str(&rest[at..at + end + 2]),
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Records into `links` where each nested settings struct is documented.
fn collect_links(meta: &Meta, prefix: &str, links: &mut Links) {
    if !prefix.is_empty() {
        let table = table_name(prefix);
        links.insert(meta.name.to_string(), (format!("`{table}`"), slug(&table)));
    }
    for f in meta.fields {
        if let FieldKind::Nested { meta: submeta } = f.kind {
            collect_links(submeta, &format!("{prefix}{}.", f.name), links);
        }
    }
}

/// One setting, as a block rather than a table row: the name and its
/// environment variable on the first line, the type badge and the description
/// on the second.
fn setting(name: &str, env: Option<&str>, badge: Markup, desc: &str) -> Markup {
    html! {
        div class="setting" {
            div class="d-flex flex-wrap align-items-baseline gap-2" {
                code class="setting-name" { (name) }
                @if let Some(env) = env {
                    span class="setting-env ms-auto" { code { (env) } }
                }
            }
            div class="setting-doc" { (value_markdown_with_lead(badge, None, desc)) }
        }
    }
}

/// Recursively renders a settings (sub-)struct: a heading and the struct's own
/// prose, the documented leaf fields, then a sub-section per nested struct. A
/// struct holding nothing but nested structs still gets its heading and prose.
fn section<'a>(
    body: &mut Body,
    meta: &Meta,
    prefix: &str,
    root: &'a Value,
    defs: Defs<'a, '_>,
    links: &Links,
) {
    let context = match prefix.is_empty() {
        true => "General".to_string(),
        false => table_name(prefix),
    };
    // The root struct's fields sit in no table of their own, so that section is
    // named rather than quoted, and its doc is the page intro rendered by the
    // caller.
    if prefix.is_empty() {
        body.heading(3, html! { "General" });
    } else {
        // A section sits one level under the page's own ("Values", "Special
        // types"), so 3..=5 by how deep its table is nested.
        let level = match prefix.trim_end_matches('.').split('.').count() {
            1 => 3,
            2 => 4,
            _ => 5,
        };
        body.name_heading(level, &table_name(prefix));
        body.markdown(&resolve(&meta.doc.join("\n"), links));
    }

    let mut untyped: Vec<String> = Vec::new();
    let settings: Vec<Markup> = meta
        .fields
        .iter()
        .filter_map(|f| {
            let FieldKind::Leaf { env, .. } = f.kind else {
                return None;
            };
            let path = format!("{prefix}{}", f.name);
            // A field the schema does not describe is one kept out of the file
            // format on purpose. It has no type to show and is not documented.
            let field = schema::field(root, &path, defs)?;
            let badge = type_badge(field, defs, Some("setting-badge"));
            if badge.is_none() {
                untyped.push(f.name.to_string());
            }
            let desc = resolve(&f.doc.join("\n"), links);
            Some(setting(&path, env, badge.unwrap_or_default(), &desc))
        })
        .collect();
    warn_untyped(&context, &untyped);

    if !settings.is_empty() {
        body.push(html! {
            div class="setting-list" { @for setting in settings { (setting) } }
        });
    }

    for f in meta.fields {
        if let FieldKind::Nested { meta: submeta } = f.kind {
            section(body, submeta, &format!("{prefix}{}.", f.name), root, defs, links);
        }
    }
}

/// The schema definitions used as the type an array or a map holds, plus the
/// ones those in turn hold in a field, as `(name, title, schema)`.
fn object_types(root: &Value) -> Vec<(&str, &str, &Value)> {
    let mut elements = BTreeSet::new();
    collect_element_refs(root, &mut elements);
    let Some(definitions) = root.get("$defs").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut referenced = BTreeSet::new();
    for name in elements {
        collect_field_defs(name, definitions, &mut referenced);
    }
    definitions
        .iter()
        .filter(|(name, _)| referenced.contains(name.as_str()))
        .map(|(name, def)| (name.as_str(), schema::title(Some(def), name), def))
        .collect()
}

/// Collects into `found` the definition `name` and every definition its fields
/// hold, transitively. Names already collected are not walked again, so cyclic
/// schemas terminate.
fn collect_field_defs<'a>(
    name: &'a str,
    definitions: &'a serde_json::Map<String, Value>,
    found: &mut BTreeSet<&'a str>,
) {
    if !found.insert(name) {
        return;
    }
    // A type documented here lists its fields, so a type one of those fields
    // holds needs a section of its own to link to. Only the fields are
    // followed: a nested settings struct is a TOML table with a section
    // already, and pulling it in here would document it twice.
    let Some(props) = definitions.get(name).and_then(|def| def.get("properties")) else {
        return;
    };
    let mut held = BTreeSet::new();
    for prop in schema::children(props) {
        collect_refs(prop, &mut held);
    }
    for name in held {
        collect_field_defs(name, definitions, found);
    }
}

/// Collects the names of every definition referenced, anywhere in the subtree,
/// as the type an array holds (`items`) or a map holds
/// (`additionalProperties`). A definition referenced in any other position is
/// not collected.
fn collect_element_refs<'a>(schema: &'a Value, found: &mut BTreeSet<&'a str>) {
    if let Some(map) = schema.as_object() {
        let held = ["items", "additionalProperties"]
            .iter()
            .filter_map(|key| map.get(*key))
            .filter_map(schema::ref_name);
        found.extend(held);
    }
    schema::children(schema).for_each(|child| collect_element_refs(child, found));
}

/// Collects the names of every definition referenced anywhere in the subtree,
/// in any position.
fn collect_refs<'a>(schema: &'a Value, found: &mut BTreeSet<&'a str>) {
    // The whole subtree is searched: an optional field keeps its `$ref` inside
    // an `anyOf`.
    found.extend(schema::ref_name(schema));
    schema::children(schema).for_each(|child| collect_refs(child, found));
}

/// Renders the object format of a type that has no TOML table of its own, being
/// the element type of an array or a map, or a type one of those holds in a
/// field.
/// Both the types and the prose come from the schema, which carries the doc
/// comments as descriptions.
fn object<'a>(body: &mut Body, title: &str, def: &'a Value, defs: Defs<'a, '_>, links: &Links) {
    body.heading(3, html! { (title) });
    if let Some(doc) = def.get("description").and_then(Value::as_str) {
        body.markdown(&resolve(doc, links));
    }
    let (table, untyped) = doc_table(def, defs, |name| {
        def.get("properties")
            .and_then(|props| props.get(name))
            .and_then(|prop| prop.get("description"))
            .and_then(Value::as_str)
            .map(|doc| resolve(doc, links))
    });
    warn_untyped(title, &untyped);
    body.push(table);
}

pub fn body() -> Body {
    let schema = schema_for!(Settings);
    let root = schema.as_value();
    let defs = |name: &str| root.get("$defs").and_then(|defs| defs.get(name));
    let objects = object_types(root);

    let mut links = Links::new();
    collect_links(&Settings::META, "", &mut links);
    for (def_name, title, _) in &objects {
        links.insert(def_name.to_string(), (title.to_string(), slug(title)));
    }

    let mut body = Body::new(html! { h1 { "Settings Reference" } });
    // Page intro (TOML file, required values, relative paths, env precedence)
    // lives on the `Settings` type.
    body.markdown(&resolve(&Settings::META.doc.join("\n"), &links));
    body.push(details("Example settings.toml", code_block(EXAMPLE, "toml")));

    body.heading(2, html! { "Values" });
    section(&mut body, &Settings::META, "", root, &defs, &links);

    body.heading(2, html! { "Special types" });
    for (_, title, def) in &objects {
        object(&mut body, title, def, &defs, &links);
    }

    body
}
