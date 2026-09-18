use documented::{Documented, DocumentedFields};
use id2202_autograder_macros::TestKind;
use schemars::JsonSchema;

use super::{discover_by_suffix, PostInit, PostInitCtx};
use crate::error::Error;

/// Execute a binary and check its output against a set of predefined expected
/// values.
///
/// For example, this can test configuration be used to test a solution on
/// `./n_primes 10` against an expected output:
///
/// ```toml
/// [test]
/// kind = "run"
///
/// [test.options]
/// bin = "n_primes"
/// args = ["10"]
///
/// # This specifies that the run program must exit with code 0.
/// code = [0]
///
/// # There is only allowed value, and stdout must match that. But we don't
/// # care about any whitespaces that the program outputs.
/// stdout = ["2,3,5,7,11,13,17,19,23,29"]
/// stdout_strip_whitespace = true
///
/// # Also don't care about stderr, any text there is accepted
/// stderr = []
/// ```
///
/// Example of checking whether output matches one of predefined outputs:
///
/// ```toml
/// [test]
/// kind = "run"
///
/// [test.options]
/// bin = "randint"
/// args = ["4"]
///
/// # Don't care about return code or stderr here
/// code = []
/// stderr = []
///
/// # `./randint 4` should generate a random integer in the range [0,4)
/// stdout = ["0", "1", "2", "3"]
/// stdout_trim = true
/// ```
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
    /// Acceptable exit codes. If an empty list is provided, then any exit
    /// code is allowed.
    pub code: Vec<i32>,
    /// Expected texts on stdout, checking that captured stdout matches any of
    /// the provided values. If an empty list is provided, then any text on
    /// stdout is allowed.
    pub stdout: Vec<String>,
    /// Trim whitespace from captured stdout before comparing against the
    /// expected values in the `stdout` list.
    pub stdout_trim: bool,
    /// Remove all whitespace characters from captured stdout before comparing
    /// against the expected values in the `stdout` list.
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
