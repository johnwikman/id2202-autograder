//! The REST API page, rendered from the OpenAPI JSON emitted by the `server`
//! binary (`server emit-openapi`): one accordion item per operation, holding
//! its parameters, its payload, an example `curl` invocation and a pane per
//! response code.

use std::collections::BTreeMap;

use actix_web::http::StatusCode;
use maud::{html, Markup};
use serde_json::{json, Map, Value};

use crate::components::{
    code_block, details, field_table, html_table, notched_box, type_badge, value_markdown,
    value_markdown_with_lead, warn_untyped, Body, FieldDoc, LOCK_ICON,
};
use crate::openapi::{Operation, RequestBody, SecurityScheme, Spec};
use crate::schema;

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

/// The HTTP reason phrase for a status code (`Not Found` for `404`). `None`
/// when it is not a standard code.
fn reason_phrase(status: &str) -> Option<&'static str> {
    status
        .parse()
        .ok()
        .and_then(|code| StatusCode::from_u16(code).ok())
        .and_then(|code| code.canonical_reason())
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

/// The security scheme documented once in the page intro instead of on every
/// endpoint that uses it. Any other scheme is documented on its own endpoints.
const GLOBAL_SCHEME: &str = "api_token";

fn security_badges(op: &Operation) -> Markup {
    html! {
        @for name in op.security_names() {
            span class="badge bg-secondary ms-2" { (LOCK_ICON) " " (name) }
        }
    }
}

/// Documents one security scheme: the badge that marks the endpoints using it,
/// `lead` as context, and how the credential is sent. The scheme's own
/// description is the authoritative prose. Only a scheme without one falls back
/// to a header line derived from its definition.
fn scheme_box(name: &str, scheme: &SecurityScheme, lead: &str) -> Markup {
    let header = match scheme.kind.as_str() {
        "http" => Some(html! { "Authorization: Bearer <token>" }),
        "apiKey" if scheme.location == "header" => Some(html! { (scheme.name) ": <value>" }),
        _ => None,
    };
    let inner = html! {
        p class="mb-0" { (lead) }
        @match (&scheme.description, header) {
            (Some(description), _) => { (value_markdown(description)) }
            (None, Some(header)) => {
                p class="mb-0 mt-2" {
                    "Send " code class="doc-code" { (header) } " with the request."
                }
            }
            (None, None) => {}
        }
    };
    notched_box(html! { (LOCK_ICON) " " (name) }, inner)
}

fn auth_intro(spec: &Spec) -> Markup {
    let Some(scheme) = spec.components.security_schemes.get(GLOBAL_SCHEME) else {
        return Markup::default();
    };
    scheme_box(
        GLOBAL_SCHEME,
        scheme,
        "Endpoints that carry this badge in their header must authenticate with an autograder \
         API token, issued by the administrator. The endpoints that authenticate differently \
         carry a different badge and document their own scheme.",
    )
}

/// The boxes documenting the schemes an operation uses that are not covered by
/// the page-level [`auth_intro`] box.
fn endpoint_auth(op: &Operation, spec: &Spec) -> Markup {
    html! {
        @for name in op.security_names().filter(|name| *name != GLOBAL_SCHEME) {
            @if let Some(scheme) = spec.components.security_schemes.get(name) {
                (scheme_box(name, scheme, "This endpoint is not authenticated with an API token."))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

fn params_table(op: &Operation, schemas: &BTreeMap<String, Value>) -> Markup {
    if op.parameters.is_empty() {
        return Markup::default();
    }
    let defs = |name: &str| schemas.get(name);
    let rows: Vec<Vec<Markup>> = op
        .parameters
        .iter()
        .map(|p| {
            vec![
                html! { code class="doc-field" { (p.name) } },
                html! { (p.location) },
                type_badge(&p.schema, &defs, None).unwrap_or_default(),
                html! { (if p.required { "yes" } else { "no" }) },
                value_markdown(&p.description),
            ]
        })
        .collect();
    html! {
        p class="fw-semibold mb-1" { "Parameters" }
        (html_table(&["Name", "In", "Type", "Required", "Description"], &rows))
    }
}

// ---------------------------------------------------------------------------
// Request body
// ---------------------------------------------------------------------------

/// The component a property holds, together with the suffix a path below it
/// carries, which is `[]` for an array of them. `None` for a property that
/// refers to no component.
fn property_target<'a>(
    prop: &'a Value,
    schemas: &'a BTreeMap<String, Value>,
) -> Option<(&'a Value, &'static str)> {
    let prop = schema::unwrap_single(prop);
    let (prop, suffix) = match prop.get("items") {
        Some(items) => (schema::unwrap_single(items), "[]"),
        None => (prop, ""),
    };
    Some((schemas.get(schema::ref_name(prop)?)?, suffix))
}

/// Appends to `rows` the rows of a payload's field table: every property of an
/// object schema, each followed by the fields of the component it refers to,
/// named by a dotted path under it (`sink.url`, `commits[].id`). A property
/// with no description of its own takes the one on the variant its `$ref` is
/// wrapped in, then the one on that component, and is marked optional when the
/// object holding it does not require it. `depth` bounds how far the nesting is
/// followed.
fn payload_fields<'a>(
    schema: &'a Value,
    schemas: &'a BTreeMap<String, Value>,
    prefix: &str,
    depth: u8,
    rows: &mut Vec<(String, &'a Value, FieldDoc)>,
) {
    let required = schema::required(schema);

    for (name, prop) in schema::properties(schema) {
        let target = property_target(prop, schemas);
        let doc = prop
            .get("description")
            .or_else(|| schema::unwrap_single(prop).get("description"))
            .or_else(|| target.and_then(|(target, _)| target.get("description")))
            .and_then(Value::as_str);
        let path = format!("{prefix}{name}");
        rows.push((
            path.clone(),
            prop,
            FieldDoc {
                doc: doc.map(str::to_string),
                note: (!required.contains(&name)).then(|| "(optional)".to_string()),
            },
        ));
        if let Some((target, suffix)) = target.filter(|_| depth > 0) {
            payload_fields(target, schemas, &format!("{path}{suffix}."), depth - 1, rows);
        }
    }
}

/// The request body an endpoint expects: a field table over the payload's own
/// keys and what the endpoint does with keys it does not list, alongside the
/// names of the fields the schema gives no type to. Empty when the operation
/// has no JSON request body. The table is left out when the payload's root
/// schema has no properties.
fn payload_section(op: &Operation, schemas: &BTreeMap<String, Value>) -> (Markup, Vec<String>) {
    let Some(request_body) = &op.request_body else {
        return (Markup::default(), Vec::new());
    };
    let Some(schema) = request_body.json_schema() else {
        return (Markup::default(), Vec::new());
    };
    let schema = schema::unwrap_single(schema);
    let target = match schema::ref_name(schema).and_then(|name| schemas.get(name)) {
        Some(target) => target,
        None => schema,
    };
    let defs = |name: &str| schemas.get(name);

    // Three levels of nested object are expanded.
    let mut rows = Vec::new();
    payload_fields(target, schemas, "", 3, &mut rows);
    let (table, untyped) = match rows.is_empty() {
        true => (Markup::default(), Vec::new()),
        false => field_table(rows, &defs),
    };

    // The spec states a description on the body only where the handler was
    // annotated with one. Otherwise the payload type's own prose stands in.
    let intro = match request_body.description.is_empty() {
        true => target.get("description").and_then(Value::as_str),
        false => Some(request_body.description.as_str()),
    };
    // A payload may carry fields its schema does not list, which is what an
    // object states by leaving `additionalProperties` out.
    let open = target.get("type").and_then(Value::as_str) == Some("object")
        && target.get("additionalProperties") != Some(&Value::Bool(false));
    let markup = html! {
        p class="fw-semibold mb-1 mt-3" { "Request body" }
        @if let Some(intro) = intro { (value_markdown(intro)) }
        @if open {
            p class="text-body-secondary mb-1" {
                "Only the listed fields are required. Payloads carrying additional fields are \
                 accepted as long as the listed fields are provided."
            }
        }
        (table)
    };
    (markup, untyped)
}

// ---------------------------------------------------------------------------
// Example request
// ---------------------------------------------------------------------------

/// A collapsed `curl` invocation of the endpoint, carrying everything it
/// requires, with a placeholder in place of every value the API client has to
/// supply.
fn curl_example(op: &Operation, spec: &Spec, method: &str, path: &str) -> Markup {
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

    let required_params =
        |location| op.parameters.iter().filter(move |p| p.required && p.location == location);
    let query: Vec<String> =
        required_params("query").map(|p| format!("{}={}", p.name, placeholder(&p.name))).collect();
    if !query.is_empty() {
        url.push('?');
        url.push_str(&query.join("&"));
    }

    let mut headers: Vec<(String, String)> =
        required_params("header").map(|p| (p.name.clone(), placeholder(&p.name))).collect();
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

    let payload = op.request_body.as_ref().and_then(RequestBody::json_schema).map(|schema| {
        let example = example_value(schema, &spec.components.schemas, EXAMPLE_NESTING);
        serde_json::to_string_pretty(&example).unwrap_or_default()
    });
    if payload.is_some() {
        headers.push(("Content-Type".to_string(), "application/json".to_string()));
    }

    let mut args = vec![match method {
        "get" => format!("curl \"{url}\""),
        _ => format!("curl -X {} \"{url}\"", method.to_uppercase()),
    }];
    args.extend(headers.iter().map(|(name, value)| format!("-H \"{name}: {value}\"")));
    if let Some(payload) = &payload {
        args.push(format!("--data '{}'", payload.replace('\'', "'\\''")));
    }
    details("Example request", code_block(&args.join(" \\\n  "), "bash"))
}

// ---------------------------------------------------------------------------
// Example values
// ---------------------------------------------------------------------------

/// How many levels of nesting a generated example expands.
const EXAMPLE_NESTING: u8 = 16;

/// Generates a structural example JSON value from a schema, resolving `$ref`
/// against the component schemas. `depth` guards against deep/recursive schemas.
fn example_value(schema: &Value, schemas: &BTreeMap<String, Value>, depth: u8) -> Value {
    if depth == 0 {
        return json!({});
    }
    let schema = schema::unwrap_single(schema);
    // An `allOf` of several branches is a conjunction, satisfied by all of them
    // at once, which is how utoipa encodes `#[serde(flatten)]`.
    if let Some(branches) = schema.get("allOf").and_then(Value::as_array) {
        let merged: Map<String, Value> = branches
            .iter()
            .filter_map(|branch| match example_value(branch, schemas, depth - 1) {
                Value::Object(fields) => Some(fields),
                _ => None,
            })
            .flatten()
            .collect();
        return Value::Object(merged);
    }
    // A union has no one shape, and an example can only show one, so it shows
    // the first variant that is not `null`.
    let variant = ["anyOf", "oneOf"].iter().find_map(|key| {
        schema
            .get(*key)
            .and_then(Value::as_array)
            .and_then(|variants| variants.iter().find(|v| schema::type_of(v) != Some("null")))
    });
    if let Some(variant) = variant {
        return example_value(variant, schemas, depth - 1);
    }
    if let Some(name) = schema::ref_name(schema) {
        return match schemas.get(name) {
            Some(target) => example_value(target, schemas, depth - 1),
            None => json!({}),
        };
    }
    match schema::type_of(schema).unwrap_or("object") {
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
// Responses
// ---------------------------------------------------------------------------

/// A strip of coloured status-code tabs, with a tab pane per code holding what
/// makes the endpoint return that code, a generated example response, and the
/// response schema.
fn responses_tabs(op: &Operation, schemas: &BTreeMap<String, Value>, op_id: &str) -> Markup {
    let panes: Vec<Markup> = op
        .responses
        .iter()
        .enumerate()
        .map(|(i, (status, resp))| {
            let badge = html! {
                span class="badge ct-badge me-2" {
                    (status)
                    @if let Some(reason) = reason_phrase(status) { " " (reason) }
                }
            };
            let intro = value_markdown_with_lead(badge, Some("mt-1 mb-3"), &resp.description);
            let schema = resp.json_schema().map(|schema| {
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
                (pretty, schema::document(schema, schemas))
            });
            html! {
                div class={
                    "tab-pane fade" (if i == 0 { " show active" } else { "" })
                    " ct-pane st-" (status_variant(status)) " rounded-bottom p-3"
                } id={ (op_id) "-" (status) } role="tabpanel" {
                    (intro)
                    @match schema {
                        Some((pretty, document)) => {
                            p class="fw-semibold mb-1" { "Example response" }
                            (code_block(&pretty, "json"))
                            (details("Schema", code_block(&document, "json")))
                        }
                        None => { p class="text-body-secondary" { "No JSON body." } }
                    }
                }
            }
        })
        .collect();

    // A tab and its pane have to carry the same `st-*` class, which the `.ct*`
    // rules in `docs.css` merge the two on.
    html! {
        div class="nav nav-tabs ct-nav" role="tablist" {
            @for (i, status) in op.responses.keys().enumerate() {
                button class={
                    "nav-link ct st-" (status_variant(status))
                    (if i == 0 { " active" } else { "" })
                } data-bs-toggle="tab" data-bs-target={ "#" (op_id) "-" (status) } type="button"
                    role="tab" { (status) }
            }
        }
        div class="tab-content" { @for pane in panes { (pane) } }
    }
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

pub fn body(spec: &Spec) -> Body {
    let schemas = &spec.components.schemas;

    let mut body = Body::new(html! { h1 { "REST API Reference" } });
    if let Some(description) = &spec.info.description {
        body.push(value_markdown(description));
    }
    body.push(auth_intro(spec));

    // The id tying each endpoint's collapsible panel to the accordion, so that
    // opening one closes the one already open.
    let accordion_id = "apiAccordion";

    // Each item mints its id from `body`, which the `body.push` writing the
    // accordion cannot borrow at the same time, so the items are gathered first
    // and the accordion wrapped around them afterwards.
    let mut items: Vec<Markup> = Vec::new();
    for (path, item) in &spec.paths {
        for (method, op) in item.operations() {
            let anchor = body.anchor(
                &format!("{method}-{path}")
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                    .collect::<String>(),
            );
            let id = anchor.id().to_string();
            // The collapsible panel is a second element of the same endpoint,
            // so its id hangs off the one the endpoint was minted, as do the
            // response tabs' below.
            let panel = format!("c-{id}");
            let upper = method.to_uppercase();
            let bg = method_bg(method);

            // The accordion header is a button rather than a heading, so the
            // submenu entry is recorded by hand against the item's own id.
            // `<wbr>` makes a path too long for the sidebar break after a `/`
            // rather than mid-segment.
            body.entry(
                2,
                &anchor,
                html! {
                    span class="api-toc" {
                        span class={ "badge " (bg) } { (upper) }
                        code {
                            @for (i, segment) in path.split('/').enumerate() {
                                @if i > 0 { "/" wbr; }
                                (segment)
                            }
                        }
                    }
                },
            );

            // `summary` is the doc comment's first line and `description` the
            // rest, so the two are put back together as it was written.
            let doc = match (&op.summary, &op.description) {
                (summary, None) => summary.clone(),
                (None, description) => description.clone(),
                (Some(summary), Some(description)) => Some(format!("{summary}\n\n{description}")),
            };

            let (payload, untyped) = payload_section(op, schemas);
            warn_untyped(&format!("{upper} {path} request body"), &untyped);

            items.push(html! {
                div class="accordion-item" id=(id) {
                    h2 class="accordion-header" {
                        button class="doc-api-btn accordion-button collapsed" type="button"
                            data-bs-toggle="collapse" data-bs-target={ "#" (panel) }
                            aria-expanded="false" {
                            span class={ "badge " (bg) " api-method" } { (upper) }
                            strong class="api-path fs-5" { code { (path) } }
                            (security_badges(op))
                        }
                    }
                    div id=(panel) class="accordion-collapse collapse"
                        data-bs-parent={ "#" (accordion_id) } {
                        div class="accordion-body" {
                            @if let Some(doc) = &doc { (value_markdown(doc)) }
                            (endpoint_auth(op, spec))
                            (params_table(op, schemas))
                            (payload)
                            (curl_example(op, spec, method, path))
                            p class="fw-semibold mb-1 mt-3" { "Response codes" }
                            (responses_tabs(op, schemas, &id))
                        }
                    }
                }
            });
        }
    }
    body.push(html! {
        div class="accordion" id=(accordion_id) { @for item in items { (item) } }
    });

    body
}
