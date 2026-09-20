use documented::{Documented, DocumentedFields};
use id2202_autograder_macros::TestKind;
use schemars::JsonSchema;

use super::{PostInit, PostInitCtx};
use crate::error::Error;

/// Simple test kind for verifying that a file exists in the submitted
/// solution, optionally checking some of its properties.
///
/// This example test verifies that a file `description.pdf` exists under
/// `solutions/file-example`, and that it is a valid PDF file:
///
/// ```toml
/// [test]
/// kind = "check_file_exists"
///
/// [test.options]
/// path = "solutions/file-example/description.pdf"
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
/// path = "solutions/file-example/test.cpp"
/// mimetype_prefix_ignore = true
/// ```
///
/// Note that the `mimetype_prefix` is usually ignored by default, but can be
/// disabled this way if a parent `config.toml` specified a specific MIME type
/// to check file.
#[derive(JsonSchema, Debug, Clone, Documented, DocumentedFields, TestKind)]
#[testkind(ident = "check_file_exists")]
pub struct CheckFileExists {
    /// Path to the file (relative to the repository root).
    pub path: String,
    /// Required MIME type prefix, e.g. `application/pdf`. Set
    /// `mimetype_prefix_ignore = true` instead to skip the check.
    #[testkind(ignorable)]
    pub mimetype_prefix: Option<String>,
}

impl PostInit for CheckFileExists {
    fn post_init(&mut self, _ctx: &PostInitCtx) -> Result<(), Error> {
        Ok(())
    }
}
