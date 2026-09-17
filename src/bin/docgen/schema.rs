//! Reading a JSON schema: the type of a value, and the standalone schema
//! document a definition expands into.
//!
//! Two kinds of schema reach this module: the `schemars` documents generated
//! from the configuration types, and the component schemas inside the OpenAPI
//! spec. A `$ref` is therefore resolved through a [`Defs`] lookup the caller
//! supplies rather than against a fixed location.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};

/// Resolves the target of a `$ref` by name.
pub type Defs<'a, 'b> = &'b dyn Fn(&str) -> Option<&'a Value>;

/// Where `schemars` keeps its definitions.
const DEFS_PREFIX: &str = "#/$defs/";

/// Where OpenAPI keeps its schemas.
const COMPONENT_PREFIX: &str = "#/components/schemas/";

/// The definition a `$ref` names, for a reference into `#/$defs/` (`schemars`)
/// or `#/components/schemas/` (OpenAPI). `None` for a schema carrying no
/// `$ref`, and for a `$ref` into anything else, such as
/// `#/components/responses/`, which holds no schemas.
pub fn ref_name(schema: &Value) -> Option<&str> {
    let reference = schema.get("$ref").and_then(Value::as_str)?;
    [DEFS_PREFIX, COMPONENT_PREFIX].iter().find_map(|prefix| reference.strip_prefix(prefix))
}

/// The name a definition is documented under: the `title` it states, and
/// otherwise `name`, which is also the answer for a `def` of `None`.
pub fn title<'a>(def: Option<&'a Value>, name: &'a str) -> &'a str {
    def.and_then(|def| def.get("title")).and_then(Value::as_str).unwrap_or(name)
}

/// The properties an object schema lists as `required`, in the order it lists
/// them.
pub fn required(schema: &Value) -> Vec<&str> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// The schemas directly under `schema`: the values of an object, the items of
/// an array. Empty for anything else.
pub fn children(schema: &Value) -> impl Iterator<Item = &Value> {
    let (fields, items) = match schema {
        Value::Object(map) => (Some(map.values()), None),
        Value::Array(items) => (None, Some(items.iter())),
        _ => (None, None),
    };
    fields.into_iter().flatten().chain(items.into_iter().flatten())
}

/// The type a schema states, with the `"null"` a nullable value lists alongside
/// it dropped. `None` when the schema states no type.
pub fn type_of(schema: &Value) -> Option<&str> {
    match schema.get("type") {
        Some(Value::String(ty)) => Some(ty.as_str()),
        Some(Value::Array(types)) => {
            types.iter().filter_map(Value::as_str).find(|ty| *ty != "null")
        }
        _ => None,
    }
}

/// A schema reduced to the single type it wraps, repeatedly: the `anyOf`/`oneOf`
/// of an optional value, which is a choice between the type and `null`, and the
/// one-branch `allOf` a `$ref` carrying its own metadata sits in. A choice
/// between several types and an `allOf` of several branches name no single type
/// and are returned unchanged.
pub fn unwrap_single(schema: &Value) -> &Value {
    if let Some(branches) = schema.get("allOf").and_then(Value::as_array) {
        return match branches.as_slice() {
            [only] => unwrap_single(only),
            _ => schema,
        };
    }
    let variants =
        ["anyOf", "oneOf"].iter().find_map(|key| schema.get(*key).and_then(Value::as_array));
    let Some(variants) = variants else {
        return schema;
    };
    let mut stated = variants.iter().filter(|variant| type_of(variant) != Some("null"));
    match (stated.next(), stated.next()) {
        (Some(only), None) => unwrap_single(only),
        _ => schema,
    }
}

/// The type of a value. `plural` asks for the form the type reads with when a
/// container holds several of them. Only a noun takes it, meaning a plain type
/// or the name a definition is documented under, so a `format` (`uint16`) and an
/// `array of …` are the same either way. `None` when the schema says nothing
/// about the type.
fn describe<'a>(schema: &'a Value, defs: Defs<'a, '_>, plural: bool) -> Option<String> {
    let s = match plural {
        true => "s",
        false => "",
    };
    let schema = unwrap_single(schema);
    if let Some(name) = ref_name(schema) {
        let target = defs(name);
        let noun = title(target, name);
        // A definition that is not an object, such as an untagged union or an
        // enum of strings, is named on its own rather than as "… object", and
        // so is one whose name is a single word rather than a prose phrase.
        let object = target.is_some_and(|target| type_of(target) == Some("object"));
        return Some(match object && noun.contains(' ') {
            true => format!("{noun} object{s}"),
            false => format!("{noun}{s}"),
        });
    }
    let ty = type_of(schema);
    if ty == Some("array") {
        let element =
            schema.get("items").and_then(|items| describe(items, defs, true)).unwrap_or_default();
        return Some(format!("array of {element}"));
    }
    // A map is an object whose keys the settings file names itself, which the
    // schema states as the type every one of them holds.
    if let Some(values) = schema.get("additionalProperties").filter(|v| v.is_object()) {
        return Some(format!("map of {}", describe(values, defs, true).unwrap_or_default()));
    }
    // The exact width and signedness of a number is worth keeping, and only the
    // format carries it, as `minimum`/`maximum` are emitted for the narrow
    // types and left off the rest.
    match schema.get("format").and_then(Value::as_str) {
        Some(format) if ty.is_some() => Some(format.to_string()),
        _ => ty.map(|ty| format!("{ty}{s}")),
    }
}

/// The type of a value as the documentation names it: `uint16`, `string`,
/// `array of strings`, `array of int32`, `GitHub instance object`. `None` when
/// the schema says nothing about the type.
pub fn type_name<'a>(schema: &'a Value, defs: Defs<'a, '_>) -> Option<String> {
    describe(schema, defs, false)
}

/// The `ty-*` class colouring a type badge, read from the schema's own type.
/// `ty-other` covers an object and anything the schema gives no type to.
fn scalar_class(schema: &Value) -> &'static str {
    match type_of(unwrap_single(schema)) {
        Some("string") => "ty-string",
        Some("integer" | "number") => "ty-integer",
        Some("boolean") => "ty-boolean",
        _ => "ty-other",
    }
}

/// The classes a type badge carries. An array takes the colour of the type it
/// holds and is additionally marked `is-array`.
pub fn type_class(schema: &Value) -> String {
    let schema = unwrap_single(schema);
    match type_of(schema) {
        Some("array") => {
            let element = schema.get("items").map(scalar_class).unwrap_or("ty-other");
            format!("{element} is-array")
        }
        _ => scalar_class(schema).to_string(),
    }
}

/// Every property of an object schema: the ones `required` lists first, in that
/// order, then the rest alphabetically.
pub fn properties(schema: &Value) -> Vec<(&str, &Value)> {
    // `serde_json` sorts `properties`, but `schemars` writes `required` in
    // declaration order, which is the order the configuration file is written
    // in.
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let required = required(schema);

    let mut fields: Vec<(&str, &Value)> =
        props.iter().map(|(name, prop)| (name.as_str(), prop)).collect();
    fields.sort_by_key(|(name, _)| {
        required.iter().position(|declared| declared == name).unwrap_or(required.len())
    });
    fields
}

/// The schema of a field at a dotted path (`submission.github.webhook_secret`),
/// following a `$ref` at each step down. `None` for a field the schema does not
/// describe, which is how a `#[schemars(skip)]` field is left undocumented.
pub fn field<'a>(root: &'a Value, path: &str, defs: Defs<'a, '_>) -> Option<&'a Value> {
    let mut current = root;
    for segment in path.split('.') {
        let resolved = ref_name(current).and_then(defs).unwrap_or(current);
        current = resolved.get("properties")?.get(segment)?;
    }
    Some(current)
}

/// Collects into `found` every definition reachable from `schema`, following
/// each one into its own references. Names already collected are not walked
/// again, so cyclic schemas terminate.
fn collect_reachable_defs<'a>(
    schema: &Value,
    definitions: &'a BTreeMap<String, Value>,
    found: &mut BTreeSet<&'a str>,
) {
    if let Some(name) = ref_name(schema) {
        if let Some((name, target)) = definitions.get_key_value(name) {
            if found.insert(name.as_str()) {
                collect_reachable_defs(target, definitions, found);
            }
        }
        return;
    }
    children(schema).for_each(|child| collect_reachable_defs(child, definitions, found));
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

fn pretty_object(entries: &[(&str, Value)]) -> String {
    // `serde_json::Map` is a `BTreeMap` here and would sort the keys, putting
    // the bulky `$defs` first.
    let fields: Vec<String> = entries
        .iter()
        .map(|(key, value)| {
            let rendered = serde_json::to_string_pretty(value).unwrap_or_default();
            format!("  \"{key}\": {}", rendered.replace('\n', "\n  "))
        })
        .collect();
    format!("{{\n{}\n}}", fields.join(",\n"))
}

/// Root-level key order of a schema document, matching what the `/api/schema/*`
/// routes serve. Keys not listed follow, alphabetically.
const KEY_ORDER: &[&str] = &["description", "type", "properties", "required"];

/// Expands a schema into a standalone JSON Schema document, in the shape the
/// `/api/schema/*` routes serve: the definition it refers to inlined at the top
/// level, and every definition that one transitively reaches gathered under
/// `$defs`. References are rewritten rather than inlined, so recursive schemas
/// stay finite.
pub fn document(schema: &Value, definitions: &BTreeMap<String, Value>) -> String {
    let (name, root) = match ref_name(schema).and_then(|name| definitions.get_key_value(name)) {
        Some((name, target)) => (Some(name.as_str()), target),
        None => (None, schema),
    };
    let Some(fields) = root.as_object() else {
        return serde_json::to_string_pretty(root).unwrap_or_default();
    };

    let mut doc = vec![("$schema", json!("https://json-schema.org/draft/2020-12/schema"))];
    if let Some(name) = name {
        doc.push(("title", json!(name)));
    }

    let mut keys: Vec<&String> = fields.keys().collect();
    keys.sort_by_key(|k| {
        KEY_ORDER.iter().position(|known| *known == k.as_str()).unwrap_or(KEY_ORDER.len())
    });
    doc.extend(keys.into_iter().map(|k| (k.as_str(), rewrite_refs(&fields[k]))));

    let mut reachable = BTreeSet::new();
    collect_reachable_defs(root, definitions, &mut reachable);
    if !reachable.is_empty() {
        let reachable: Map<String, Value> = reachable
            .into_iter()
            .map(|name| (name.to_string(), rewrite_refs(&definitions[name])))
            .collect();
        doc.push(("$defs", Value::Object(reachable)));
    }

    pretty_object(&doc)
}
