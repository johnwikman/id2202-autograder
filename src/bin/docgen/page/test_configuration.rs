//! The test configuration reference. `documented` yields the struct- and
//! field-level doc comments of each test kind; the field types come from the
//! same type's `schemars` schema.

use documented::{Documented, DocumentedFields};
use schemars::{schema_for, JsonSchema};
use serde_json::Value;

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

use crate::html::{html_page, Body};
use crate::page::common::{doc_table, name_heading, FieldDoc};

/// How an option key behaves beyond carrying a value. Written here rather than
/// in the doc comments so that every kind phrases it the same way, and so that
/// changing an attribute cannot leave the prose behind.
fn behavior(f: &FieldAttrs) -> String {
    let mut out = String::new();
    if f.is_relpath {
        out.push_str(" Resolved against the directory of the file that sets it.");
    }
    if !f.clears.is_empty() {
        let cleared: Vec<String> = f.clears.iter().map(|c| format!("`{c}`")).collect();
        out.push_str(&format!(
            " Setting this key resets {} to the default.",
            cleared.join(", ")
        ));
    }
    if f.deep_merge {
        out.push_str(" Inherited entries are merged key by key, not replaced wholesale.");
    }
    out
}

fn testkind_section<T>(body: &mut Body, heading: &str, attrs: &[FieldAttrs])
where
    T: Documented + DocumentedFields + JsonSchema,
{
    body.heading(3, &name_heading(heading));
    body.markdown(T::DOCS);

    let schema = schema_for!(T);
    let schema = schema.as_value();
    // A test kind is a flat struct, so nothing in it is a reference.
    let defs = |_: &str| None::<&Value>;
    body.raw(&doc_table(heading, schema, &defs, |name| {
        let mut doc = match T::get_field_docs(name) {
            Ok(doc) => doc.to_string(),
            Err(_) => String::new(),
        };
        let attrs = attrs.iter().find(|f| f.name == name);
        if let Some(f) = attrs {
            doc.push_str(&behavior(f));
        }
        FieldDoc {
            doc,
            note: attrs
                .and_then(|f| f.ignore_key)
                .map(|key| format!("(Disable with `{key} = true`)")),
        }
    }));
}

pub fn render(name: &str) -> String {
    let mut body = Body::new("<h1>Test Configuration Reference</h1>\n");
    // Overview + hierarchical inheritance prose lives on the `Tests` type.
    body.markdown(Tests::DOCS);

    body.heading(2, "Root defaults (<code>[default]</code>)");
    testkind_section::<TagDefaults>(&mut body, "[default.tag]", &[]);
    testkind_section::<BuildConfig>(&mut body, "[default.build]", &[]);
    testkind_section::<TestConfig>(&mut body, "[default.test]", &[]);

    body.heading(2, "Test kinds");
    testkind_section::<TestkindRun>(&mut body, "run", TestkindRun::FIELDS);
    testkind_section::<TestkindGenASMAndRun>(
        &mut body,
        "gen_asm_and_run",
        TestkindGenASMAndRun::FIELDS,
    );
    testkind_section::<TestkindCheckFileExists>(
        &mut body,
        "check_file_exists",
        TestkindCheckFileExists::FIELDS,
    );
    testkind_section::<TestkindRunVerifier>(&mut body, "run_verifier", TestkindRunVerifier::FIELDS);

    html_page(name, "Test Configuration Reference", "tests.html", body)
}
