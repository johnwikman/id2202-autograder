//! Reading the type of a value out of a JSON schema.
//!
//! Two kinds of schema reach this module — the `schemars` documents generated
//! from the configuration types, and the component schemas inside the OpenAPI
//! spec — so a reference is resolved through a `defs` lookup the caller
//! supplies rather than against a fixed location.

use serde_json::Value;

/// Resolves the target of a `$ref` by name.
pub type Defs<'a, 'b> = &'b dyn Fn(&str) -> Option<&'a Value>;

/// The name a `$ref` points at, which is its last path segment. Covers both
/// `#/$defs/X` (`schemars`) and `#/components/schemas/X` (OpenAPI).
fn ref_name(schema: &Value) -> Option<&str> {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.rsplit('/').next())
}

/// A schema with any leading `$ref` followed.
fn resolve<'a>(schema: &'a Value, defs: Defs<'a, '_>) -> &'a Value {
    match ref_name(schema).and_then(defs) {
        Some(target) => target,
        None => schema,
    }
}

/// The type of a value, and whether that name pluralises when it is an array's
/// element type. A `format` is a name (`uint16`), so it does not; a plain type
/// or a definition's title is a noun, so it does.
fn describe<'a>(schema: &'a Value, defs: Defs<'a, '_>) -> (String, bool) {
    if let Some(name) = ref_name(schema) {
        let title = defs(name)
            .and_then(|target| target.get("title"))
            .and_then(Value::as_str);
        return match title {
            Some(title) => (format!("{title} object"), true),
            None => ("object".to_string(), true),
        };
    }
    // The type may be a bare string or, for a nullable value, an array holding
    // the type alongside "null".
    let ty = match schema.get("type") {
        Some(Value::String(ty)) => ty.as_str(),
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .find(|ty| *ty != "null")
            .unwrap_or_default(),
        _ => "",
    };
    if ty == "array" {
        let element = match schema.get("items") {
            Some(items) => describe(items, defs),
            None => (String::new(), false),
        };
        let (name, plural) = element;
        let name = match plural {
            true => format!("{name}s"),
            false => name,
        };
        return (format!("array of {name}"), false);
    }
    // The exact width and signedness of a number is worth keeping, and only the
    // format carries it — `minimum`/`maximum` are emitted for the narrow types
    // and left off the rest.
    match schema.get("format").and_then(Value::as_str) {
        Some(format) if !ty.is_empty() => (format.to_string(), false),
        _ => (ty.to_string(), true),
    }
}

/// The type of a value as the documentation names it: `uint16`, `string`,
/// `array of strings`, `array of int32`, `GitHub instance object`. Empty when
/// the schema says nothing about the type.
pub fn type_name<'a>(schema: &'a Value, defs: Defs<'a, '_>) -> String {
    describe(schema, defs).0
}

/// The `ty-*` class colouring a type badge. Taken from the schema's own type
/// rather than the displayed name, which may be a format (`uint16`) or a title.
/// `ty-other` covers an object and anything the schema gives no type to.
fn scalar_class(schema: &Value) -> &'static str {
    match schema.get("type").and_then(Value::as_str) {
        Some("string") => "ty-string",
        Some("integer") | Some("number") => "ty-integer",
        Some("boolean") => "ty-boolean",
        _ => "ty-other",
    }
}

/// The classes a type badge carries. An array takes the colour of the type it
/// holds — so `array of strings` scans as a string setting — and is marked
/// `is-array` for the rails that tell the two apart.
pub fn type_class(schema: &Value) -> String {
    match schema.get("type").and_then(Value::as_str) {
        Some("array") => {
            let element = schema.get("items").map(scalar_class).unwrap_or("ty-other");
            format!("{element} is-array")
        }
        _ => scalar_class(schema).to_string(),
    }
}

/// Every property of an object schema, in the order the fields are declared.
/// JSON Schema does not order `properties` (and `serde_json` sorts them), but
/// `schemars` writes `required` in declaration order, which is the order the
/// configuration file is written in. Anything not required keeps the sorted
/// order, after them.
pub fn properties(schema: &Value) -> Vec<(&str, &Value)> {
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut fields: Vec<(&str, &Value)> = props
        .iter()
        .map(|(name, prop)| (name.as_str(), prop))
        .collect();
    fields.sort_by_key(|(name, _)| {
        required
            .iter()
            .position(|declared| declared == name)
            .unwrap_or(required.len())
    });
    fields
}

/// The schema of a field at a dotted path (`submission.github.webhook_secret`),
/// following a `$ref` at each step down. `None` for a field the schema does not
/// describe, which is how a `#[schemars(skip)]` field is left undocumented.
pub fn field<'a>(root: &'a Value, path: &str, defs: Defs<'a, '_>) -> Option<&'a Value> {
    let mut current = root;
    for segment in path.split('.') {
        current = resolve(current, defs).get("properties")?.get(segment)?;
    }
    Some(current)
}
