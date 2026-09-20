//! The Python package description `griffe dump` writes.
//!
//! An annotation keeps its `serde_json` form, which saves a variant per kind of
//! expression node.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

/// What `griffe dump` writes, keyed by package name.
pub type Dump = BTreeMap<String, Object>;

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Module,
    Class,
    Function,
    Attribute,
    /// Documented under the definition it names.
    Alias,
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
pub struct Docstring {
    pub value: String,
}

#[derive(Deserialize)]
pub struct Parameter {
    pub name: String,
    #[serde(default)]
    pub annotation: Option<Value>,
    #[serde(default)]
    pub default: Option<Value>,
}

#[derive(Deserialize)]
pub struct Decorator {
    pub value: Value,
}

#[derive(Deserialize)]
pub struct Object {
    pub kind: Kind,
    pub name: String,
    #[serde(default)]
    pub lineno: u32,
    #[serde(default)]
    pub docstring: Option<Docstring>,
    #[serde(default)]
    pub members: BTreeMap<String, Object>,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    pub returns: Option<Value>,
    #[serde(default)]
    pub bases: Vec<Value>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub decorators: Vec<Decorator>,
    #[serde(default)]
    pub annotation: Option<Value>,
    #[serde(default)]
    pub value: Option<Value>,
}

impl Object {
    pub fn docs(&self) -> &str {
        self.docstring.as_ref().map_or("", |doc| doc.value.as_str())
    }

    /// In declaration order. `griffe` keys members by name, which would
    /// otherwise list a class's fields alphabetically.
    pub fn members_of(&self, kind: Kind) -> Vec<&Object> {
        let mut out: Vec<&Object> = self
            .members
            .values()
            .filter(|member| member.kind == kind && !member.name.starts_with('_'))
            .collect();
        out.sort_by_key(|member| member.lineno);
        out
    }

    /// The definitions `__all__` names, in its order. Looked up in the
    /// package's own submodules only.
    pub fn exports(&self) -> Vec<&Object> {
        let Some(names) = self
            .members
            .get("__all__")
            .and_then(|all| all.value.as_ref())
            .and_then(|value| value.get("elements"))
            .and_then(Value::as_array)
        else {
            eprintln!("warning: griffe: package `{}` states no __all__", self.name);
            return Vec::new();
        };
        names
            .iter()
            .filter_map(|element| {
                let name = source_text(element);
                let name = name.trim_matches(['\'', '"']);
                let found = self
                    .members
                    .values()
                    .filter(|member| member.kind == Kind::Module)
                    .find_map(|module| module.members.get(name))
                    .filter(|object| object.kind != Kind::Alias);
                if found.is_none() {
                    eprintln!(
                        "warning: griffe: `{}` exports `{name}`, which no submodule defines",
                        self.name
                    );
                }
                found
            })
            .collect()
    }
}

/// The source text an annotation was written as. `griffe` describes one as a
/// tree of expression nodes, or as a plain string where it needs no structure.
pub fn source_text(expr: &Value) -> String {
    if let Value::String(text) = expr {
        return text.clone();
    }
    let part = |key: &str| expr.get(key).map(source_text).unwrap_or_default();
    let joined = |key: &str, sep: &str| match expr.get(key).and_then(Value::as_array) {
        Some(items) => items.iter().map(source_text).collect::<Vec<_>>().join(sep),
        None => String::new(),
    };
    match expr.get("cls").and_then(Value::as_str) {
        Some("ExprName") => part("name"),
        Some("ExprConstant") => part("value"),
        Some("ExprAttribute") => joined("values", "."),
        Some("ExprSubscript") => format!("{}[{}]", part("left"), part("slice")),
        Some("ExprTuple") => joined("elements", ", "),
        Some("ExprList") => format!("[{}]", joined("elements", ", ")),
        Some("ExprBinOp") => format!("{} {} {}", part("left"), part("operator"), part("right")),
        Some("ExprCall") => format!("{}({})", part("function"), joined("arguments", ", ")),
        Some("ExprKeyword") => format!("{}={}", part("name"), part("value")),
        Some("ExprStarred") => format!("*{}", part("value")),
        // Silence here would drop a piece of an annotation, such as the
        // members of a `Literal`.
        Some(node) => {
            eprintln!("warning: griffe: no rendering for expression node `{node}`");
            String::new()
        }
        None => String::new(),
    }
}
