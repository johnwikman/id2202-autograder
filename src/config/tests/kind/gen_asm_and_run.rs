use documented::{Documented, DocumentedFields};
use id2202_autograder_macros::TestKind;
use schemars::JsonSchema;

use super::{discover_by_suffix, PostInit, PostInitCtx};
use crate::error::Error;

/// This test kind grades the solution using a multi-stage pipeline:
///
///  1. run the student binary to generate an assembly file,
///  2. assemble it,
///  3. compile it, then
///  4. run the compiled binary.
///
/// The output from each stage is checked along the way, only proceeding to the
/// next stage if the previous one was successful. The checking of stdout,
/// stderr, and exit codes follow the same semantics as for the `run` test
/// kind.
///
/// For example, given the following test group:
///
/// ```text
/// my-group/
///   config.toml
///   genasm-example.test.toml
///   genasm-example.c
/// ```
///
/// Then in `genasm-example.test.toml`:
///
/// ```toml
/// [test]
/// kind = "gen_asm_and_run"
///
/// [test.options]
/// bin = "student-compiler"
/// args = ["--emit-x86_64"]
/// code = [0]
/// auto_input_files = [".c"]
///
/// assemble_cmd = ["nasm", "-felf64", "<ASM_FILE>", "-o", "sol.o"]
/// assemble_code = [0]
/// compile_cmd = ["gcc", "-no-pie", "sol.o", "-o", "sol.exe"]
/// compile_code = [0]
///
/// run_cmd = ["./sol.exe", "echo", "5"]
/// run_stdout = ["5\n"]
/// ```
///
/// This will copy the file `genasm-example.c` into the container, and provide
/// a path to it as a trailing argument to the run command. This test will
/// approximately be run as:
///
///  1. `/solution/student-compiler --emit-x86_64 PATH_TO_C_FILE > out.asm`
///  2. `nasm -felf64 ./out.asm -o sol.o`
///  3. `gcc -no-pie sol.o -o sol.exe`
///  4. `./sol.exe echo 5`
///
/// The semantics of the `run` test kind applies to the final stage, where
/// stdout, stderr, and exit code is captured, and then checked against valid
/// alternatives.
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
    /// Files to copy into the container and provide as input to program.
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
