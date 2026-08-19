//! The REST API page, rendered from the OpenAPI JSON emitted by the `server`
//! binary (`server emit-openapi`).
//!
//! The spec has to come from the server because the handler annotations live
//! there, but all HTML rendering happens here. Only the parts of the spec the
//! page actually shows are modelled below; everything else is ignored. Schemas
//! stay as raw [`Value`], since the two things done with them — generating an
//! example and re-assembling a standalone schema document — both only need to
//! walk the JSON, not understand all of JSON Schema.

use std::collections::{BTreeMap, BTreeSet};

use actix_web::http::StatusCode;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::html::{code_block, details, html_page, html_table, notched_box, Body, LOCK_ICON};
use crate::markdown::{escape, inline};
use crate::page::common::type_badge;

// ---------------------------------------------------------------------------
// Spec model
// ---------------------------------------------------------------------------

/// `BTreeMap` throughout, so paths and status codes render in a stable
/// alphabetical order regardless of how the server emitted them.
#[derive(Default, Deserialize)]
pub struct Spec {
    #[serde(default)]
    info: Info,
    #[serde(default)]
    paths: BTreeMap<String, PathItem>,
    #[serde(default)]
    components: Components,
}

#[derive(Default, Deserialize)]
struct Info {
    #[serde(default)]
    description: String,
}

#[derive(Default, Deserialize)]
struct Components {
    #[serde(default)]
    schemas: BTreeMap<String, Value>,
    #[serde(default, rename = "securitySchemes")]
    security_schemes: BTreeMap<String, SecurityScheme>,
}

#[derive(Deserialize)]
struct SecurityScheme {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    description: String,
    /// The header name, for `apiKey` schemes.
    #[serde(default)]
    name: String,
    #[serde(default, rename = "in")]
    location: String,
}

#[derive(Default, Deserialize)]
struct PathItem {
    get: Option<Operation>,
    post: Option<Operation>,
    put: Option<Operation>,
    delete: Option<Operation>,
    patch: Option<Operation>,
    head: Option<Operation>,
    options: Option<Operation>,
    trace: Option<Operation>,
}

impl PathItem {
    fn operations(&self) -> impl Iterator<Item = (&'static str, &Operation)> {
        [
            ("get", &self.get),
            ("post", &self.post),
            ("put", &self.put),
            ("delete", &self.delete),
            ("patch", &self.patch),
            ("head", &self.head),
            ("options", &self.options),
            ("trace", &self.trace),
        ]
        .into_iter()
        .filter_map(|(method, op)| op.as_ref().map(|op| (method, op)))
    }
}

#[derive(Deserialize)]
struct Operation {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    parameters: Vec<Param>,
    #[serde(default)]
    responses: BTreeMap<String, Response>,
    /// Each entry is one alternative set of schemes; the values (OAuth scopes)
    /// are unused here, so only the keys are read.
    #[serde(default)]
    security: Vec<BTreeMap<String, Value>>,
}

impl Operation {
    fn security_names(&self) -> impl Iterator<Item = &str> {
        self.security.iter().flat_map(|req| req.keys().map(String::as_str))
    }
}

#[derive(Deserialize)]
struct Param {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "in")]
    location: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    schema: Value,
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    description: String,
    #[serde(default)]
    content: BTreeMap<String, MediaType>,
}

impl Response {
    fn json_schema(&self) -> Option<&Value> {
        self.content
            .get("application/json")
            .map(|media| &media.schema)
            .filter(|schema| !schema.is_null())
    }
}

#[derive(Deserialize)]
struct MediaType {
    #[serde(default)]
    schema: Value,
}

// ---------------------------------------------------------------------------
// Small mappings
// ---------------------------------------------------------------------------

fn method_bg(method: &str) -> &'static str {
    match method {
        "get" => "bg-success",
        "post" => "bg-primary",
        "put" => "bg-warning text-dark",
        "delete" => "bg-danger",
        "patch" => "bg-info text-dark",
        _ => "bg-secondary",
    }
}

fn status_variant(status: &str) -> &'static str {
    match status.chars().next() {
        Some('2') => "success",
        Some('3') => "info",
        Some('4') => "warning",
        Some('5') => "danger",
        _ => "secondary",
    }
}

/// The HTTP reason phrase for a status code, or `""` when it is not a standard
/// code, so a pane reads "404 Not Found" rather than "404".
fn reason_phrase(status: &str) -> &'static str {
    status
        .parse()
        .ok()
        .and_then(|code| StatusCode::from_u16(code).ok())
        .and_then(|code| code.canonical_reason())
        .unwrap_or_default()
}

/// The id of one endpoint's accordion item, also its link target.
fn endpoint_id(method: &str, path: &str) -> String {
    format!("{method}-{path}")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

/// The security scheme documented once in the page intro instead of on every
/// endpoint that uses it. Any other scheme is documented on its own endpoints.
const GLOBAL_SCHEME: &str = "api_token";

/// The accordion holding the endpoints. Each endpoint names it as its
/// `data-bs-parent`, which is what keeps only one of them open at a time.
const ACCORDION_ID: &str = "apiAccordion";

fn security_badges(op: &Operation) -> String {
    op.security_names()
        .map(|name| {
            format!("<span class=\"badge bg-secondary ms-2\">{LOCK_ICON} {}</span>", escape(name))
        })
        .collect()
}

/// Documents one security scheme: the badge that marks the endpoints using it,
/// `lead` as context, and how the credential is sent. The scheme's own
/// description is the authoritative prose; only a scheme without one falls back
/// to a header line derived from its definition.
fn scheme_box(name: &str, scheme: &SecurityScheme, lead: &str) -> String {
    let mut inner = format!("<p class=\"mb-0\">{lead}</p>\n");
    if scheme.description.is_empty() {
        let header = match scheme.kind.as_str() {
            "http" => "Authorization: Bearer &lt;token&gt;".to_string(),
            "apiKey" if scheme.location == "header" => {
                format!("{}: &lt;value&gt;", escape(&scheme.name))
            }
            _ => String::new(),
        };
        if !header.is_empty() {
            inner.push_str(&format!(
                "<p class=\"mb-0 mt-2\">Send <code>{header}</code> with the request.</p>\n"
            ));
        }
    } else {
        inner.push_str(&format!("<p class=\"mb-0 mt-2\">{}</p>\n", inline(&scheme.description)));
    }
    notched_box(&format!("{LOCK_ICON} {}", escape(name)), &inner)
}

fn auth_intro(spec: &Spec) -> String {
    match spec.components.security_schemes.get(GLOBAL_SCHEME) {
        Some(scheme) => scheme_box(
            GLOBAL_SCHEME,
            scheme,
            "Endpoints that carry this badge in their header must \
             authenticate with an autograder API token, issued by the \
             administrator. The endpoints that authenticate differently carry \
             a different badge and document their own scheme.",
        ),
        None => String::new(),
    }
}

/// The boxes documenting the schemes an operation uses that are not covered by
/// the page-level [`auth_intro`] box.
fn endpoint_auth(op: &Operation, spec: &Spec) -> String {
    op.security_names()
        .filter(|name| *name != GLOBAL_SCHEME)
        .filter_map(|name| {
            let scheme = spec.components.security_schemes.get(name)?;
            Some(scheme_box(name, scheme, "This endpoint is not authenticated with an API token."))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Parameters and responses
// ---------------------------------------------------------------------------

fn params_table(op: &Operation, schemas: &BTreeMap<String, Value>) -> String {
    if op.parameters.is_empty() {
        return String::new();
    }
    let defs = |name: &str| schemas.get(name);
    let rows: Vec<Vec<String>> = op
        .parameters
        .iter()
        .map(|p| {
            vec![
                format!("<code>{}</code>", escape(&p.name)),
                escape(&p.location),
                type_badge(&p.schema, &defs),
                if p.required { "yes" } else { "no" }.to_string(),
                inline(&p.description),
            ]
        })
        .collect();
    format!(
        "<p class=\"fw-semibold mb-1\">Parameters</p>\n{}",
        html_table(&["Name", "In", "Type", "Required", "Description"], &rows)
    )
}

/// Generates a structural example JSON value from a schema, resolving `$ref`
/// against the component schemas. `depth` guards against deep/recursive schemas.
fn example_value(schema: &Value, schemas: &BTreeMap<String, Value>, depth: u8) -> Value {
    if depth == 0 {
        return json!({});
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return match reference.rsplit('/').next().and_then(|n| schemas.get(n)) {
            Some(target) => example_value(target, schemas, depth - 1),
            None => json!({}),
        };
    }
    // utoipa emits `allOf`/`anyOf`/`oneOf` for `$ref` + metadata and for
    // optionals; take the first meaningful (non-null) subschema.
    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(sub) = schema
            .get(key)
            .and_then(Value::as_array)
            .and_then(|a| a.iter().find(|s| s.get("type").and_then(Value::as_str) != Some("null")))
        {
            return example_value(sub, schemas, depth);
        }
    }
    // `type` may be a string or, for nullable fields (OpenAPI 3.1), an array.
    let ty = match schema.get("type") {
        Some(Value::String(s)) => s.as_str(),
        Some(Value::Array(a)) => {
            a.iter().filter_map(Value::as_str).find(|t| *t != "null").unwrap_or("object")
        }
        _ => "object",
    };
    match ty {
        "string" => json!("string"),
        "integer" => json!(0),
        "number" => json!(0.0),
        "boolean" => json!(true),
        "array" => {
            let item = schema
                .get("items")
                .map(|it| example_value(it, schemas, depth - 1))
                .unwrap_or(Value::Null);
            json!([item])
        }
        _ => {
            let mut obj = Map::new();
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for (key, sub) in props {
                    obj.insert(key.clone(), example_value(sub, schemas, depth - 1));
                }
            } else if let Some(add) = schema.get("additionalProperties").filter(|v| v.is_object()) {
                // A map type: show a single sample entry to convey the value shape.
                obj.insert("<name>".to_string(), example_value(add, schemas, depth - 1));
            }
            Value::Object(obj)
        }
    }
}

// ---------------------------------------------------------------------------
// Schema documents
// ---------------------------------------------------------------------------

const COMPONENT_PREFIX: &str = "#/components/schemas/";
const DEFS_PREFIX: &str = "#/$defs/";

/// Root-level key order of a schema document, matching what the `/api/schema/*`
/// routes serve. Keys not listed follow, alphabetically.
const KEY_ORDER: &[&str] = &["description", "type", "properties", "required"];

/// The component name a `$ref` points at, for refs into `components/schemas`.
fn ref_name(schema: &Value) -> Option<&str> {
    schema.get("$ref").and_then(Value::as_str).and_then(|r| r.strip_prefix(COMPONENT_PREFIX))
}

/// Collects every component schema reachable from `schema` into `found`. Names
/// already collected are not walked again, so cyclic schemas terminate.
fn collect_refs<'a>(
    schema: &Value,
    schemas: &'a BTreeMap<String, Value>,
    found: &mut BTreeSet<&'a str>,
) {
    if let Some(name) = ref_name(schema) {
        if let Some((name, target)) = schemas.get_key_value(name) {
            if found.insert(name.as_str()) {
                collect_refs(target, schemas, found);
            }
        }
        return;
    }
    match schema {
        Value::Object(map) => map.values().for_each(|v| collect_refs(v, schemas, found)),
        Value::Array(items) => items.iter().for_each(|v| collect_refs(v, schemas, found)),
        _ => {}
    }
}

/// Rewrites `#/components/schemas/X` references to `#/$defs/X`, so a document
/// resolves against its own `$defs` rather than the spec it was cut out of.
fn rewrite_refs(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| match (key.as_str(), value.as_str()) {
                    ("$ref", Some(r)) => {
                        let target = match r.strip_prefix(COMPONENT_PREFIX) {
                            Some(name) => format!("{DEFS_PREFIX}{name}"),
                            None => r.to_string(),
                        };
                        (key.clone(), Value::String(target))
                    }
                    _ => (key.clone(), rewrite_refs(value)),
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(rewrite_refs).collect()),
        _ => schema.clone(),
    }
}

/// Pretty-prints an object from an ordered key list. Needed because
/// `serde_json::Map` is a `BTreeMap` here and would sort the keys, putting the
/// bulky `$defs` first.
fn pretty_object(entries: &[(&str, Value)]) -> String {
    let fields: Vec<String> = entries
        .iter()
        .map(|(key, value)| {
            let rendered = serde_json::to_string_pretty(value).unwrap_or_default();
            format!("  \"{key}\": {}", rendered.replace('\n', "\n  "))
        })
        .collect();
    format!("{{\n{}\n}}", fields.join(",\n"))
}

/// Expands a response schema into a standalone JSON Schema document, in the
/// shape the `/api/schema/*` routes serve: the referenced component inlined at
/// the top level, and every component it transitively reaches gathered under
/// `$defs`. References are rewritten rather than inlined, so recursive schemas
/// stay finite.
fn schema_document(schema: &Value, schemas: &BTreeMap<String, Value>) -> String {
    let (title, root) = match ref_name(schema).and_then(|name| schemas.get_key_value(name)) {
        Some((name, target)) => (Some(name.as_str()), target),
        None => (None, schema),
    };
    let Some(fields) = root.as_object() else {
        return serde_json::to_string_pretty(root).unwrap_or_default();
    };

    let mut doc = vec![("$schema", json!("https://json-schema.org/draft/2020-12/schema"))];
    if let Some(title) = title {
        doc.push(("title", json!(title)));
    }

    let mut keys: Vec<&String> = fields.keys().collect();
    keys.sort_by_key(|k| {
        KEY_ORDER.iter().position(|known| *known == k.as_str()).unwrap_or(KEY_ORDER.len())
    });
    doc.extend(keys.into_iter().map(|k| (k.as_str(), rewrite_refs(&fields[k]))));

    let mut defs = BTreeSet::new();
    collect_refs(root, schemas, &mut defs);
    if !defs.is_empty() {
        let defs: Map<String, Value> =
            defs.into_iter().map(|name| (name.to_string(), rewrite_refs(&schemas[name]))).collect();
        doc.push(("$defs", Value::Object(defs)));
    }

    pretty_object(&doc)
}

/// A strip of coloured status-code tabs, with a tab pane per code holding what
/// makes the endpoint return that code, a generated example response, and the
/// response schema.
fn responses_tabs(op: &Operation, schemas: &BTreeMap<String, Value>, op_id: &str) -> String {
    // The `st-*` class carries the status colour and is shared by a tab and its
    // pane; see the `.ct*` rules in `docs.css` for how the two are made to
    // merge. The Tab plugin drives any toggle carrying `role="tab"`.
    let mut nav = String::from("<div class=\"nav nav-tabs ct-nav\" role=\"tablist\">\n");
    let mut panes = String::from("<div class=\"tab-content\">\n");

    for (i, (status, resp)) in op.responses.iter().enumerate() {
        let tab = format!("{op_id}-{status}");
        let (nav_active, pane_active) = match i {
            0 => (" active", " show active"),
            _ => ("", ""),
        };
        let variant = status_variant(status);
        nav.push_str(&format!(
            "<button class=\"nav-link ct st-{variant}{nav_active}\" data-bs-toggle=\"tab\" \
             data-bs-target=\"#{tab}\" type=\"button\" role=\"tab\">{status}</button>\n",
        ));

        panes.push_str(&format!(
            "<div class=\"tab-pane fade{pane_active} ct-pane st-{variant} rounded-bottom p-3\" \
             id=\"{tab}\" role=\"tabpanel\">\n"
        ));
        // The status badge repeats the tab in the pane, filled with the same
        // colour, so the sentence after it reads as what makes the endpoint
        // return that code rather than as a caption for the example below.
        let reason = reason_phrase(status);
        let label = match reason.is_empty() {
            true => status.to_string(),
            false => format!("{status} {reason}"),
        };
        panes.push_str(&format!(
            "<p class=\"mt-1 mb-3\"><span class=\"badge ct-badge me-2\">{label}</span>{}</p>\n",
            inline(&resp.description)
        ));

        match resp.json_schema() {
            Some(schema) => {
                let mut example = example_value(schema, schemas, 16);
                // A body that carries its own status code (RFC 9457 problem
                // details) shows the code of the pane it sits in, rather than
                // the placeholder every integer otherwise gets.
                if let (Some(obj), Ok(code)) = (example.as_object_mut(), status.parse::<u16>()) {
                    if let Some(v @ Value::Number(_)) = obj.get_mut("status") {
                        *v = json!(code);
                    }
                }
                let pretty = serde_json::to_string_pretty(&example).unwrap_or_default();
                panes.push_str("<p class=\"fw-semibold mb-1\">Example response</p>\n");
                panes.push_str(&code_block(&pretty, "json"));
                // Collapsed: the schema document is many times the height of
                // the example, and is the thing a reader goes looking for
                // rather than one they read on the way past.
                panes.push_str(&details(
                    "Schema",
                    &code_block(&schema_document(schema, schemas), "json"),
                ));
            }
            None => panes.push_str("<p class=\"text-body-secondary\">No JSON body.</p>\n"),
        }
        panes.push_str("</div>\n");
    }

    nav.push_str("</div>\n");
    panes.push_str("</div>\n");
    format!("{nav}{panes}")
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

pub fn render(name: &str, spec: &Spec) -> String {
    let schemas = &spec.components.schemas;

    let mut body = Body::new("<h1>REST API Reference</h1>\n");
    if !spec.info.description.is_empty() {
        body.raw(&format!("<p>{}</p>\n", inline(&spec.info.description)));
    }
    body.raw(&auth_intro(spec));

    body.raw(&format!("<div class=\"accordion\" id=\"{ACCORDION_ID}\">\n"));
    for (path, item) in &spec.paths {
        for (method, op) in item.operations() {
            let id = endpoint_id(method, path);
            let upper = method.to_uppercase();
            let bg = method_bg(method);

            // The accordion header is a button, not a heading docgen can give
            // an anchor to, so the endpoint is listed in the submenu by hand and
            // the item itself carries the id the entry links to. `REVEAL_JS`
            // opens whichever item the fragment points at.
            // A path too long for the sidebar breaks after a `/` rather than
            // mid-segment, which is what the browser would otherwise do.
            body.entry(
                2,
                &id,
                &format!(
                    "<span class=\"api-toc\"><span class=\"badge {bg}\">{upper}</span>\
                     <code>{}</code></span>",
                    escape(path).replace('/', "/<wbr>")
                ),
            );

            // The method badge is absolutely positioned (`.api-method`) into a
            // fixed left slot, the path (`.api-path`) carries the aligning
            // margin plus `margin-right:auto` to push the auth badge right.
            body.raw(&format!("<div class=\"accordion-item\" id=\"{id}\">\n"));
            body.raw(&format!(
                "<h2 class=\"accordion-header\">\
                 <button class=\"accordion-button collapsed\" type=\"button\" \
                 data-bs-toggle=\"collapse\" data-bs-target=\"#c-{id}\" aria-expanded=\"false\">\
                 <span class=\"badge {bg} api-method\">{upper}</span>\
                 <strong class=\"api-path fs-5\"><code>{path}</code></strong>{sec}</button>\
                 </h2>\n",
                path = escape(path),
                sec = security_badges(op),
            ));
            // `data-bs-parent` is what makes the accordion close whichever
            // endpoint was open when another is expanded.
            body.raw(&format!(
                "<div id=\"c-{id}\" class=\"accordion-collapse collapse\" \
                 data-bs-parent=\"#{ACCORDION_ID}\">\n<div class=\"accordion-body\">\n"
            ));

            let desc = match op.description.is_empty() {
                true => &op.summary,
                false => &op.description,
            };
            if !desc.is_empty() {
                body.raw(&format!("<p>{}</p>\n", inline(desc)));
            }
            body.raw(&endpoint_auth(op, spec));
            body.raw(&params_table(op, schemas));
            body.raw("<p class=\"fw-semibold mb-1 mt-3\">Response codes</p>\n");
            body.raw(&responses_tabs(op, schemas, &id));

            body.raw("</div>\n</div>\n</div>\n");
        }
    }
    body.raw("</div>\n");

    html_page(name, "REST API Reference", "api.html", body)
}
