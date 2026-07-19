//! The test configuration reference. `documented` yields the struct- and
//! field-level doc comments of each test kind; the field types come from the
//! same type's `schemars` schema.

use documented::{Documented, DocumentedFields};
use schemars::{schema_for, JsonSchema};
use serde_json::Value;

use id2202_autograder::config::tests::{
    TestDefault, TestkindCheckFileExists, TestkindGenASMAndRun, TestkindRun, Tests,
};

use crate::html::{html_page, Body};
use crate::page::common::{collapse_ws, doc_table, name_heading};

fn testkind_section<T>(body: &mut Body, heading: &str)
where
    T: Documented + DocumentedFields + JsonSchema,
{
    body.heading(3, &name_heading(heading));
    body.markdown(T::DOCS);

    let schema = schema_for!(T);
    let schema = schema.as_value();
    // A test kind is a flat struct, so nothing in it is a reference.
    let defs = |_: &str| None::<&Value>;
    body.raw(&doc_table(
        heading,
        schema,
        &defs,
        |name| match T::get_field_docs(name) {
            Ok(doc) => collapse_ws(doc),
            Err(_) => String::new(),
        },
    ));
}

pub fn render(name: &str) -> String {
    let mut body = Body::new("<h1>Test Configuration Reference</h1>\n");
    // Overview + hierarchical inheritance prose lives on the `Tests` type.
    body.markdown(Tests::DOCS);

    body.heading(2, "Root defaults (<code>[default]</code>)");
    testkind_section::<TestDefault>(&mut body, "[default]");

    body.heading(2, "Test kinds");
    testkind_section::<TestkindRun>(&mut body, "run");
    testkind_section::<TestkindGenASMAndRun>(&mut body, "gen_asm_and_run");
    testkind_section::<TestkindCheckFileExists>(&mut body, "check_file_exists");

    html_page(name, "Test Configuration Reference", "tests.html", body)
}
