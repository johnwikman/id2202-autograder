use documented::{Documented, DocumentedFields};
use id2202_autograder_macros::TestKind;
use schemars::JsonSchema;

use super::{discover_by_suffix, PostInit, PostInitCtx};
use crate::error::Error;

/// Execute a binary and check its output.
#[derive(JsonSchema, Debug, Clone, Documented, DocumentedFields, TestKind)]
#[testkind(ident = "run")]
pub struct Run {
    /// Binary to execute.
    pub bin: String,
    /// Command-line arguments.
    pub args: Vec<String>,
    /// Optional text to write on stdin.
    #[testkind(ignorable)]
    pub stdin: Option<String>,
    /// Acceptable exit codes.
    pub code: Vec<i32>,
    /// Expected stdout lines.
    pub stdout: Vec<String>,
    /// Trim whitespace from each line before comparing.
    pub stdout_trim: bool,
    /// Strip all whitespace before comparing.
    pub stdout_strip_whitespace: bool,
    /// Expected stderr lines.
    pub stderr: Vec<String>,
    /// Trim whitespace from each stderr line.
    pub stderr_trim: bool,
    /// Strip all whitespace from stderr.
    pub stderr_strip_whitespace: bool,
    /// Files to copy into the container.
    pub input_files: Vec<String>,

    /// Suffixes for automatically discovering input files,
    /// e.g. `[".cpp"]`.
    pub auto_input_files: Vec<String>,
}

impl PostInit for Run {
    fn post_init(&mut self, ctx: &PostInitCtx) -> Result<(), Error> {
        discover_by_suffix(&mut self.input_files, &self.auto_input_files, ctx)
    }
}
