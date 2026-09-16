//! The REST API page, rendered from the OpenAPI JSON emitted by the `server`
//! binary (`server emit-openapi`).
//!
//! The spec has to come from the server because the handler annotations live
//! there, but all HTML rendering happens here. Only the parts of the spec the
//! page actually shows are modelled below; everything else is ignored. Schemas
//! stay as raw [`Value`], since the three things done with them — generating an
//! example, re-assembling a standalone schema document and listing a request
//! body's fields — all only need to walk the JSON, not understand all of JSON
//! Schema.

use std::collections::{BTreeMap, BTreeSet};

use actix_web::http::StatusCode;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::html::{code_block, details, html_page, html_table, notched_box, Body, LOCK_ICON};
use crate::markdown::escape;
use crate::page::common::{field_table, type_badge, value_markdown, FieldDoc};
use crate::schema;

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
    #[serde(default, rename = "requestBody")]
    request_body: Option<RequestBody>,
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
        json_schema(&self.content)
    }
}

#[derive(Deserialize)]
struct RequestBody {
    #[serde(default)]
    description: String,
    #[serde(default)]
    content: BTreeMap<String, MediaType>,
}

impl RequestBody {
    fn json_schema(&self) -> Option<&Value> {
        json_schema(&self.content)
    }
}

/// The schema `content` states for `application/json`, or `None` when it has no
/// such entry or that entry carries no schema.
fn json_schema(content: &BTreeMap<String, MediaType>) -> Option<&Value> {
    content.get("application/json").map(|media| &media.schema).filter(|schema| !schema.is_null())
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
                "<p class=\"mb-0 mt-2\">Send <code class=\"doc-code\">{header}</code> \
                 with the request.</p>\n"
            ));
        }
    } else {
        inner.push_str(&value_markdown(&scheme.description));
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
// Parameters
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
                type_badge(&p.schema, &defs, ""),
                if p.required { "yes" } else { "no" }.to_string(),
                value_markdown(&p.description),
            ]
        })
        .collect();
    format!(
        "<p class=\"fw-semibold mb-1\">Parameters</p>\n{}",
        html_table(&["Name", "In", "Type", "Required", "Description"], &rows)
    )
}

// ---------------------------------------------------------------------------
// Request body
// ---------------------------------------------------------------------------

/// How many levels of nested object a request body's field table expands.
const BODY_NESTING: u8 = 3;

/// A schema with any wrapper utoipa puts around a `$ref` — `allOf` for one
/// carrying metadata, `anyOf`/`oneOf` with `null` for an optional one — taken
/// off, leaving the first subschema that is not `null`. A genuine union is not
/// handled: every variant after the first is dropped.
fn unwrap_variant(schema: &Value) -> &Value {
    for key in ["allOf", "anyOf", "oneOf"] {
        let variant = schema.get(key).and_then(Value::as_array).and_then(|variants| {
            variants.iter().find(|v| v.get("type").and_then(Value::as_str) != Some("null"))
        });
        if let Some(variant) = variant {
            return unwrap_variant(variant);
        }
    }
    schema
}

/// The component a property holds, together with the suffix a path below it
/// carries — `[]` for an array of them. `None` for a property that refers to no
/// component.
fn property_target<'a>(
    prop: &'a Value,
    schemas: &'a BTreeMap<String, Value>,
) -> Option<(&'a Value, &'static str)> {
    let prop = unwrap_variant(prop);
    let (prop, suffix) = match prop.get("items") {
        Some(items) => (unwrap_variant(items), "[]"),
        None => (prop, ""),
    };
    Some((schemas.get(ref_name(prop)?)?, suffix))
}

/// Appends to `rows` the rows of a request body's field table: every property
/// of an object schema, each followed by the fields of the component it refers
/// to, named by a dotted path under it (`sink.url`, `commits[].id`). A property
/// with no description of its own takes the one on the variant its `$ref` is
/// wrapped in, then the one on that component, and is marked optional when the
/// object holding it does not require it. `depth` bounds how far the nesting is
/// followed.
fn body_fields<'a>(
    schema: &'a Value,
    schemas: &'a BTreeMap<String, Value>,
    prefix: &str,
    depth: u8,
    rows: &mut Vec<(String, &'a Value, FieldDoc)>,
) {
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    for (name, prop) in schema::properties(schema) {
        let target = property_target(prop, schemas);
        let doc = prop
            .get("description")
            .or_else(|| unwrap_variant(prop).get("description"))
            .or_else(|| target.and_then(|(target, _)| target.get("description")))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let path = format!("{prefix}{name}");
        rows.push((
            path.clone(),
            prop,
            FieldDoc {
                doc: doc.to_string(),
                note: (!required.contains(&name)).then(|| "(optional)".to_string()),
            },
        ));
        if let Some((target, suffix)) = target.filter(|_| depth > 0) {
            body_fields(target, schemas, &format!("{path}{suffix}."), depth - 1, rows);
        }
    }
}

/// The body an endpoint expects: a field table over the payload's own keys and
/// what the endpoint does with keys it does not list. Empty when the operation
/// has no JSON request body; the table is left out when that body's root schema
/// has no properties.
fn request_body(
    op: &Operation,
    schemas: &BTreeMap<String, Value>,
    method: &str,
    path: &str,
) -> String {
    let Some(body) = &op.request_body else {
        return String::new();
    };
    let Some(schema) = body.json_schema() else {
        return String::new();
    };
    let target = match ref_name(schema).and_then(|name| schemas.get(name)) {
        Some(target) => target,
        None => schema,
    };
    let defs = |name: &str| schemas.get(name);

    let mut rows = Vec::new();
    body_fields(target, schemas, "", BODY_NESTING, &mut rows);
    let table = match rows.is_empty() {
        true => String::new(),
        false => {
            field_table(&format!("{} {path} request body", method.to_uppercase()), rows, &defs)
        }
    };

    // The spec states a description on the body only where the handler was
    // annotated with one; otherwise the payload type's own prose stands in.
    let intro = match body.description.is_empty() {
        true => target.get("description").and_then(Value::as_str).unwrap_or_default(),
        false => &body.description,
    };
    let description = match intro.is_empty() {
        true => String::new(),
        false => value_markdown(intro),
    };
    let subset = match accepts_unknown_fields(target) {
        true => "<p class=\"text-body-secondary mb-1\">Only the fields listed are read. A \
                 payload carrying others is still accepted; the rest is ignored.</p>\n",
        false => "",
    };
    format!("<p class=\"fw-semibold mb-1 mt-3\">Request body</p>\n{description}{subset}{table}")
}

/// Whether a body may carry fields its schema does not list, which is what an
/// object states by leaving `additionalProperties` out.
fn accepts_unknown_fields(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("object")
        && schema.get("additionalProperties") != Some(&Value::Bool(false))
}

// ---------------------------------------------------------------------------
// Example request
// ---------------------------------------------------------------------------

/// A collapsed `curl` invocation of the endpoint. The host and every value the
/// caller has to supply are placeholders named after what they stand for; path
/// parameters are substituted, required query parameters appended, required
/// header parameters and the credential each security scheme asks for sent as
/// `-H`, and the generated example JSON passed as the body where the endpoint
/// takes one.
fn curl_example(op: &Operation, spec: &Spec, method: &str, path: &str) -> String {
    // A header's `X-` prefix carries nothing for a reader filling the value in,
    // and `<x-direct-secret>` reads worse than `<direct_secret>`.
    let placeholder = |name: &str| {
        let lower = name.to_ascii_lowercase();
        format!("<{}>", lower.strip_prefix("x-").unwrap_or(&lower).replace('-', "_"))
    };

    let mut url = String::from("https://<host>");
    let mut rest = path;
    while let Some((before, after)) = rest.split_once('{') {
        let Some((name, tail)) = after.split_once('}') else {
            break;
        };
        url.push_str(before);
        url.push_str(&placeholder(name));
        rest = tail;
    }
    url.push_str(rest);

    let required = |location| {
        op.parameters.iter().filter(move |p| p.required && p.location == location)
    };
    let query: Vec<String> =
        required("query").map(|p| format!("{}={}", p.name, placeholder(&p.name))).collect();
    if !query.is_empty() {
        url.push('?');
        url.push_str(&query.join("&"));
    }

    let mut headers: Vec<(String, String)> =
        required("header").map(|p| (p.name.clone(), placeholder(&p.name))).collect();
    for name in op.security_names() {
        let Some(scheme) = spec.components.security_schemes.get(name) else {
            continue;
        };
        let header = match scheme.kind.as_str() {
            "http" => ("Authorization".to_string(), "Bearer <token>".to_string()),
            "apiKey" if scheme.location == "header" => {
                (scheme.name.clone(), placeholder(&scheme.name))
            }
            _ => continue,
        };
        // An endpoint may also list its credential among the parameters, as the
        // direct submission one does, and it is only sent once.
        if !headers.iter().any(|(sent, _)| sent.eq_ignore_ascii_case(&header.0)) {
            headers.push(header);
        }
    }

    let body = op.request_body.as_ref().and_then(RequestBody::json_schema).map(|schema| {
        let example = example_value(schema, &spec.components.schemas, EXAMPLE_NESTING);
        serde_json::to_string_pretty(&example).unwrap_or_default()
    });
    if body.is_some() {
        headers.push(("Content-Type".to_string(), "application/json".to_string()));
    }

    let mut args = vec![match method {
        "get" => format!("curl \"{url}\""),
        _ => format!("curl -X {} \"{url}\"", method.to_uppercase()),
    }];
    args.extend(headers.iter().map(|(name, value)| format!("-H \"{name}: {value}\"")));
    if let Some(body) = &body {
        args.push(format!("--data '{}'", body.replace('\'', "'\\''")));
    }
    details("Example request", &code_block(&args.join(" \\\n  "), "bash"))
}

// ---------------------------------------------------------------------------
// Examples and schema documents
// ---------------------------------------------------------------------------

/// How many levels of nesting a generated example expands.
const EXAMPLE_NESTING: u8 = 16;

/// Generates a structural example JSON value from a schema, resolving `$ref`
/// against the component schemas. `depth` guards against deep/recursive schemas.
fn example_value(schema: &Value, schemas: &BTreeMap<String, Value>, depth: u8) -> Value {
    if depth == 0 {
        return json!({});
    }
    let schema = unwrap_variant(schema);
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return match reference.rsplit('/').next().and_then(|n| schemas.get(n)) {
            Some(target) => example_value(target, schemas, depth - 1),
            None => json!({}),
        };
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

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

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
        let badge = format!("<span class=\"badge ct-badge me-2\">{label}</span>");
        let blocks = value_markdown(&resp.description);
        panes.push_str(&match blocks.strip_prefix("<p>") {
            Some(rest) => format!("<p class=\"mt-1 mb-3\">{badge}{rest}"),
            None => format!("<p class=\"mt-1 mb-3\">{badge}</p>\n{blocks}"),
        });

        match resp.json_schema() {
            Some(schema) => {
                let mut example = example_value(schema, schemas, EXAMPLE_NESTING);
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
        body.raw(&value_markdown(&spec.info.description));
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
                 <button class=\"doc-api-btn accordion-button collapsed\" type=\"button\" \
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

            // `summary` is the doc comment's first line and `description` the
            // rest, so the two are put back together as it was written.
            let doc = match (op.summary.is_empty(), op.description.is_empty()) {
                (_, true) => op.summary.clone(),
                (true, _) => op.description.clone(),
                _ => format!("{}\n\n{}", op.summary, op.description),
            };
            if !doc.is_empty() {
                body.raw(&value_markdown(&doc));
            }
            body.raw(&endpoint_auth(op, spec));
            body.raw(&params_table(op, schemas));
            body.raw(&request_body(op, schemas, method, path));
            body.raw(&curl_example(op, spec, method, path));
            body.raw("<p class=\"fw-semibold mb-1 mt-3\">Response codes</p>\n");
            body.raw(&responses_tabs(op, schemas, &id));

            body.raw("</div>\n</div>\n</div>\n");
        }
    }
    body.raw("</div>\n");

    html_page(name, "REST API Reference", "api.html", body)
}
