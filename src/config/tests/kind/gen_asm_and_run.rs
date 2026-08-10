use documented::{Documented, DocumentedFields};
use id2202_autograder_macros::TestKind;
use schemars::JsonSchema;

use super::{discover_by_suffix, PostInit, PostInitCtx};
use crate::error::Error;

/// Multi-stage pipeline: run the student binary to generate an assembly file,
/// assemble it, compile it, then run the compiled binary. The output from each
/// stage is checked along the way, only proceeding to the next stage if the
/// previous one was successful.
#[derive(JsonSchema, Debug, Clone, Documented, DocumentedFields, TestKind)]
#[testkind(ident = "gen_asm_and_run")]
pub struct GenASMAndRun {
    /// Binary to execute to produce the assembly.
    pub bin: String,
    /// Arguments.
    pub args: Vec<String>,
    /// Data piped to stdin. Set `stdin_ignore = true` instead to provide none.
    #[testkind(ignorable)]
    pub stdin: Option<String>,
    /// Acceptable exit codes.
    pub code: Vec<i32>,
    /// Expected stderr lines.
    pub stderr: Vec<String>,
    /// Trim stderr lines.
    pub stderr_trim: bool,
    /// Strip all whitespace from stderr.
    pub stderr_strip_whitespace: bool,
    /// Files to copy into the container.
    pub input_files: Vec<String>,
    /// Suffixes for automatically discovering input files,
    /// e.g. `[".cpp"]`.
    pub auto_input_files: Vec<String>,

    /// Assembler command. `<ASM_FILE>` is replaced with the
    /// path of the generated assembly file.
    pub assemble_cmd: Vec<String>,
    /// Acceptable assembler exit codes.
    pub assemble_code: Vec<i32>,

    /// Compiler/linker command.
    pub compile_cmd: Vec<String>,
    /// Acceptable compiler exit codes.
    pub compile_code: Vec<i32>,

    /// Command to run the compiled binary.
    pub run_cmd: Vec<String>,
    /// Data piped to stdin of the compiled binary. Set
    /// `run_stdin_ignore = true` instead to provide none.
    #[testkind(ignorable)]
    pub run_stdin: Option<String>,
    /// Acceptable exit codes.
    pub run_code: Vec<i32>,
    /// Expected stdout lines.
    pub run_stdout: Vec<String>,
    /// Trim stdout lines.
    pub run_stdout_trim: bool,
    /// Strip all whitespace from stdout.
    pub run_stdout_strip_whitespace: bool,
    /// Expected stderr lines.
    pub run_stderr: Vec<String>,
    /// Trim stderr lines.
    pub run_stderr_trim: bool,
    /// Strip all whitespace from stderr.
    pub run_stderr_strip_whitespace: bool,
}

impl PostInit for GenASMAndRun {
    fn post_init(&mut self, ctx: &PostInitCtx) -> Result<(), Error> {
        discover_by_suffix(&mut self.input_files, &self.auto_input_files, ctx)
    }
}
