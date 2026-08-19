//! The settings reference. `confique`'s `Meta` yields the nesting structure and
//! the environment-variable name alongside each field; the `schemars` schema of
//! the same types yields what `Meta` does not — the type of each value, and the
//! element type of a collection.
//!
//! That second point is why the schema is needed at all: a
//! `Vec<GitHubServerSettings>` is an opaque leaf to `Meta`, so the types that
//! appear only inside an array cannot be reached through it. They are found in
//! the schema instead, and so no list of them is kept anywhere.

use std::collections::{BTreeMap, BTreeSet};

use confique::meta::{FieldKind, Meta};
use confique::Config as _;
use schemars::schema_for;
use serde_json::Value;

use id2202_autograder::config::settings::Settings;

use crate::html::{code_block, details, html_page, slug, Body};
use crate::markdown::{collapse_ws, escape, inline};
use crate::page::common::{doc_table, name_heading, type_badge, warn_untyped};
use crate::schema::{self, Defs};

/// The repository's example settings file, embedded at build time and shown on
/// the page so the reference below has a complete file to be read against.
const EXAMPLE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/example/settings.toml"));

/// Where a doc comment's `[TypeName]` link resolves to, by type name: the
/// markdown the link shows, and the id of the section it points at.
type Links = BTreeMap<String, (String, String)>;

/// Heading level (3..=5) for a section at the given dotted-path prefix. These
/// sit one level under the page's own sections ("Values", "Special types").
fn heading_level(prefix: &str) -> usize {
    match prefix.trim_end_matches('.').split('.').count() {
        0 | 1 => 3,
        2 => 4,
        _ => 5,
    }
}

/// The TOML table a nested settings struct is configured under.
fn table_name(prefix: &str) -> String {
    format!("[{}]", prefix.trim_end_matches('.'))
}

/// Rewrites the rustdoc intra-doc links in a doc comment (`[ServerSettings]`)
/// into markdown links to the section documenting that type. Lets a doc comment
/// name the Rust type — which `cargo doc` resolves and checks — while the site
/// shows the TOML table it corresponds to. A name that is not a documented type
/// is left exactly as it was.
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
            // Already a markdown link of its own; leave it to the renderer.
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

/// Records where each nested settings struct is documented, so that a doc
/// comment referring to the type resolves to its section.
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
/// environment variable on the first line, the type and the description on the
/// second. A table cannot hold these names — the longest is 56 characters and
/// cannot be broken, so it decides the width of its column on its own.
fn setting(name: &str, env: Option<&str>, badge: &str, desc: &str) -> String {
    let env = match env {
        Some(env) => {
            format!("<span class=\"setting-env ms-auto\"><code>{}</code></span>", escape(env))
        }
        None => String::new(),
    };
    format!(
        "<div class=\"setting\">\
         <div class=\"d-flex flex-wrap align-items-baseline gap-2\">\
         <code class=\"setting-name\">{}</code>{env}</div>\
         <div class=\"setting-doc\">{badge}{}</div></div>\n",
        escape(name),
        inline(desc)
    )
}

/// Recursively renders a settings (sub-)struct: a heading and the struct's own
/// prose, the documented leaf fields, then a sub-section per nested struct. A
/// struct holding nothing but nested structs still gets its heading and prose —
/// it is a table in the settings file like any other.
fn section<'a>(
    body: &mut Body,
    meta: &Meta,
    prefix: &str,
    root: &'a Value,
    defs: Defs<'a, '_>,
    links: &Links,
) {
    // The root struct's fields sit in no table of their own, so that section is
    // named rather than quoted, and its doc is the page intro rendered by the
    // caller.
    let context = match prefix.is_empty() {
        true => {
            body.heading(3, "General");
            "General".to_string()
        }
        false => {
            body.heading(heading_level(prefix), &name_heading(&table_name(prefix)));
            body.markdown(&resolve(&meta.doc.join("\n"), links));
            table_name(prefix)
        }
    };

    let settings: String = meta
        .fields
        .iter()
        .filter_map(|f| {
            let FieldKind::Leaf { env, .. } = f.kind else {
                return None;
            };
            let path = format!("{prefix}{}", f.name);
            // A field the schema does not describe is one kept out of the file
            // format on purpose; it has no type to show and is not documented.
            let field = schema::field(root, &path, defs)?;
            let badge = type_badge(field, defs);
            if badge.is_empty() {
                warn_untyped(&context, f.name);
            }
            let desc = resolve(&collapse_ws(&f.doc.join(" ")), links);
            Some(setting(&path, env, &badge, &desc))
        })
        .collect();

    if !settings.is_empty() {
        body.raw("<div class=\"setting-list\">\n");
        body.raw(&settings);
        body.raw("</div>\n");
    }

    for f in meta.fields {
        if let FieldKind::Nested { meta: submeta } = f.kind {
            section(body, submeta, &format!("{prefix}{}.", f.name), root, defs, links);
        }
    }
}

/// The schema definitions used as the element type of an array — the object
/// formats that `confique`'s metadata cannot reach — as `(name, title, schema)`.
fn object_types(root: &Value) -> Vec<(&str, &str, &Value)> {
    let mut referenced = BTreeSet::new();
    collect_item_refs(root, &mut referenced);
    let Some(defs) = root.get("$defs").and_then(Value::as_object) else {
        return Vec::new();
    };
    defs.iter()
        .filter(|(name, _)| referenced.contains(name.as_str()))
        .map(|(name, def)| {
            let title = def.get("title").and_then(Value::as_str).unwrap_or(name.as_str());
            (name.as_str(), title, def)
        })
        .collect()
}

/// Collects the names of every definition a schema references as an array's
/// element type.
fn collect_item_refs<'a>(schema: &'a Value, found: &mut BTreeSet<&'a str>) {
    match schema {
        Value::Object(map) => {
            let item_ref = map
                .get("items")
                .and_then(|items| items.get("$ref"))
                .and_then(Value::as_str)
                .and_then(|reference| reference.rsplit('/').next());
            if let Some(name) = item_ref {
                found.insert(name);
            }
            map.values().for_each(|v| collect_item_refs(v, found));
        }
        Value::Array(items) => items.iter().for_each(|v| collect_item_refs(v, found)),
        _ => {}
    }
}

/// Renders the object format of a type reachable only inside an array (e.g. a
/// single GitHub/GitLab instance). Both the types and the prose come from the
/// schema, which carries the doc comments as descriptions.
fn object<'a>(body: &mut Body, title: &str, def: &'a Value, defs: Defs<'a, '_>, links: &Links) {
    body.heading(3, &escape(title));
    if let Some(doc) = def.get("description").and_then(Value::as_str) {
        body.markdown(&resolve(doc, links));
    }
    body.raw(&doc_table(title, def, defs, |name| {
        let doc = def
            .get("properties")
            .and_then(|props| props.get(name))
            .and_then(|prop| prop.get("description"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        resolve(doc, links)
    }));
}

pub fn render(name: &str) -> String {
    let schema = schema_for!(Settings);
    let root = schema.as_value();
    let defs = |name: &str| root.get("$defs").and_then(|defs| defs.get(name));
    let objects = object_types(root);

    let mut links = Links::new();
    collect_links(&Settings::META, "", &mut links);
    for (def_name, title, _) in &objects {
        links.insert(def_name.to_string(), (title.to_string(), slug(title)));
    }

    let mut body = Body::new("<h1>Settings Reference</h1>\n");
    // Page intro (TOML file, required values, relative paths, env precedence)
    // lives on the `Settings` type.
    body.markdown(&resolve(&Settings::META.doc.join("\n"), &links));
    body.raw(&details("Example settings.toml", &code_block(EXAMPLE, "toml")));

    body.heading(2, "Values");
    section(&mut body, &Settings::META, "", root, &defs, &links);

    body.heading(2, "Special types");
    for (_, title, def) in &objects {
        object(&mut body, title, def, &defs, &links);
    }

    html_page(name, "Settings Reference", "settings.html", body)
}
