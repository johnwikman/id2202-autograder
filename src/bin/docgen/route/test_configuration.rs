//! The test configuration reference: the root defaults, then one section per
//! test kind. `documented` yields the struct- and field-level doc comments of
//! each type shown, and the field types come from the same type's `schemars`
//! schema.

use documented::{Documented, DocumentedFields};
use maud::html;
use schemars::{schema_for, JsonSchema};

use id2202_autograder::config::tests::{
    group::TestConfig,
    kind::{
        check_file_exists::CheckFileExists as TestkindCheckFileExists,
        gen_asm_and_run::GenASMAndRun as TestkindGenASMAndRun, run::Run as TestkindRun,
        run_verifier::RunVerifier as TestkindRunVerifier, FieldAttrs, TestKind,
    },
    tag::{BuildConfig, TagDefaults},
    Tests,
};

use crate::components::{doc_table, warn_untyped, Body, FieldDoc};

/// What an option key does beyond carrying a value, as sentences to append to
/// its description. Each opens with a space, and an attribute-less key gives an
/// empty string.
fn behaviour(f: &FieldAttrs) -> String {
    let mut out = String::new();
    if f.is_relpath {
        out.push_str(
            " Resolved against the directory of the `config.toml` or `*.test.toml` \
             that sets it. A value set in the root test configuration is used as \
             written.",
        );
    }
    if !f.clears.is_empty() {
        let cleared: Vec<String> = f.clears.iter().map(|c| format!("`{c}`")).collect();
        out.push_str(&format!(
            " Setting this key discards any inherited {}, unless this file sets \
             them itself.",
            cleared.join(", ")
        ));
    }
    if f.deep_merge {
        out.push_str(
            " Inherited entries are merged key by key, not replaced wholesale. A \
             child cannot remove an inherited key.",
        );
    }
    out
}

fn type_section<T>(body: &mut Body, heading: &str, attrs: &[FieldAttrs])
where
    T: Documented + DocumentedFields + JsonSchema,
{
    body.name_heading(3, heading);
    body.markdown(T::DOCS);

    let schema = schema_for!(T);
    let schema = schema.as_value();
    let defs = |name: &str| schema.get("$defs").and_then(|defs| defs.get(name));
    let (table, untyped) = doc_table(schema, &defs, |name| {
        let attr = attrs.iter().find(|f| f.name == name);
        let doc = T::get_field_docs(name)
            .ok()
            .map(|doc| format!("{doc}{}", attr.map(behaviour).unwrap_or_default()));
        FieldDoc {
            doc,
            note: attr
                .and_then(|f| f.ignore_key)
                .map(|key| format!("(Disable with `{key} = true`)")),
        }
    });
    warn_untyped(heading, &untyped);
    body.push(table);
}

pub fn body() -> Body {
    let mut body = Body::new(html! { h1 { "Test Configuration Reference" } });
    // Overview + hierarchical inheritance prose lives on the `Tests` type.
    body.markdown(Tests::DOCS);

    body.heading(2, html! { "Root defaults (" code { "[default]" } ")" });
    type_section::<TagDefaults>(&mut body, "[default.tag]", &[]);
    type_section::<BuildConfig>(&mut body, "[default.build]", &[]);
    type_section::<TestConfig>(&mut body, "[default.test]", &[]);

    body.heading(2, html! { "Test kinds" });
    type_section::<TestkindRun>(&mut body, "run", TestkindRun::FIELDS);
    type_section::<TestkindGenASMAndRun>(
        &mut body,
        "gen_asm_and_run",
        TestkindGenASMAndRun::FIELDS,
    );
    type_section::<TestkindCheckFileExists>(
        &mut body,
        "check_file_exists",
        TestkindCheckFileExists::FIELDS,
    );
    type_section::<TestkindRunVerifier>(&mut body, "run_verifier", TestkindRunVerifier::FIELDS);

    body
}
