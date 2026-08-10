use documented::{Documented, DocumentedFields};
use id2202_autograder_macros::TestKind;
use schemars::JsonSchema;

use super::{PostInit, PostInitCtx};
use crate::error::Error;

/// Verify that a file exists, optionally checking its MIME type.
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
