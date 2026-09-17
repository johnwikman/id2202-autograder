//! The OpenAPI spec as the documentation reads it.
//!
//! Only the parts of the spec the generated pages actually show are modelled;
//! everything else is ignored. Schemas stay as raw [`Value`].

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

// `BTreeMap` throughout, so paths and status codes render in a stable
// alphabetical order regardless of how the server emitted them.
#[derive(Deserialize)]
pub struct Spec {
    #[serde(default)]
    pub info: Info,
    #[serde(default)]
    pub paths: BTreeMap<String, PathItem>,
    #[serde(default)]
    pub components: Components,
}

#[derive(Default, Deserialize)]
pub struct Info {
    pub description: Option<String>,
}

#[derive(Default, Deserialize)]
pub struct Components {
    #[serde(default)]
    pub schemas: BTreeMap<String, Value>,
    #[serde(default, rename = "securitySchemes")]
    pub security_schemes: BTreeMap<String, SecurityScheme>,
}

#[derive(Deserialize)]
pub struct SecurityScheme {
    #[serde(default, rename = "type")]
    pub kind: String,
    pub description: Option<String>,
    /// The header name, for `apiKey` schemes.
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "in")]
    pub location: String,
}

#[derive(Deserialize)]
pub struct PathItem {
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
    pub fn operations(&self) -> impl Iterator<Item = (&'static str, &Operation)> {
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
pub struct Operation {
    pub summary: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Vec<Param>,
    #[serde(default, rename = "requestBody")]
    pub request_body: Option<RequestBody>,
    #[serde(default)]
    pub responses: BTreeMap<String, Response>,
    /// Each entry is one alternative set of schemes. The values (OAuth scopes)
    /// are unused here, so only the keys are read.
    #[serde(default)]
    security: Vec<BTreeMap<String, Value>>,
}

impl Operation {
    pub fn security_names(&self) -> impl Iterator<Item = &str> {
        self.security.iter().flat_map(|req| req.keys().map(String::as_str))
    }
}

#[derive(Deserialize)]
pub struct Param {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "in")]
    pub location: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub schema: Value,
}

#[derive(Deserialize)]
pub struct Response {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    content: BTreeMap<String, MediaType>,
}

impl Response {
    pub fn json_schema(&self) -> Option<&Value> {
        json_schema(&self.content)
    }
}

#[derive(Deserialize)]
pub struct RequestBody {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    content: BTreeMap<String, MediaType>,
}

impl RequestBody {
    pub fn json_schema(&self) -> Option<&Value> {
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
