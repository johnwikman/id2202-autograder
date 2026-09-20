use documented::{Documented, DocumentedFields};
use id2202_autograder_macros::TestKind;
use schemars::JsonSchema;

use super::{PostInit, PostInitCtx};
use crate::error::Error;

/// Simple test kind for verifying that a file exists in the submitted
/// solution, optionally checking some of its properties.
///
/// # Note
/// This test kind does not use the `timeout` or `max_output` test case
/// settings since it does not execute a student's program.
///
/// This example test verifies that a file `description.pdf` exists in the
/// tag's build source directory, and that it is a valid PDF file:
///
/// ```toml
/// [test]
/// kind = "check_file_exists"
///
/// [test.options]
/// path = "description.pdf"
/// mimetype_prefix = "application/pdf"
/// ```
///
/// To just check that a file `test.cpp` exists without looking at what it
/// actually contains, the `mimetype_prefix` option can be ignored:
///
/// ```toml
/// [test]
/// kind = "check_file_exists"
///
/// [test.options]
/// path = "test.cpp"
/// mimetype_prefix_ignore = true
/// ```
///
/// The second form is what to write when the root configuration or a parent
/// `config.toml` set a MIME type to check for, and this test case only needs
/// to know that the file is there.
#[derive(JsonSchema, Debug, Clone, Documented, DocumentedFields, TestKind)]
#[testkind(ident = "check_file_exists")]
pub struct CheckFileExists {
    /// Path to the file, relative to the tag's build source directory
    /// (`build.srcdir`). A directory counts as existing, and symlinks are
    /// followed.
    pub path: String,
    /// Required MIME type prefix, e.g. `application/pdf`.
    #[testkind(ignorable)]
    pub mimetype_prefix: Option<String>,
}

impl PostInit for CheckFileExists {
    fn post_init(&mut self, _ctx: &PostInitCtx) -> Result<(), Error> {
        Ok(())
    }
}
