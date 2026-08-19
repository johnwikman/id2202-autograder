/// This file contains the functionality used to grade a test case.
use std::{collections::BTreeMap, io::Read, io::Write, path::Path, time::Duration};

use id2202_autograder::{
    config::{
        tests::kind::{
            check_file_exists::CheckFileExists as TestkindCheckFileExists,
            gen_asm_and_run::GenASMAndRun as TestkindGenASMAndRun, run::Run as TestkindRun,
            run_verifier::RunVerifier as TestkindRunVerifier,
        },
        Settings, Test,
    },
    error::{Error, ErrorKind, SyscommandError},
    reporting::{DetailsTestFailure, MIMETypeInfo, MismatchInfo, SourceFileInfo},
    utils::{self, path_absolute_join, write_all_timeout, SyscommandOutput},
};

use crate::subrunner::{
    container::{self, ContainerInfo},
    verifier,
};

/// Scratch directory inside the container for the multi-stage assembly pipeline.
const GRADING_DIR: &str = "/tmp/grading";

/// Cause of a failure
#[derive(Debug, Clone)]
pub enum FailureCause {
    OutputMismatch,
    Timeout(Duration),
    OutputLimitExceeded { limit: usize },
}

/// Treats stdout and stderr to the format that we expect.
fn treat_output(s: &str, trim: bool, remove_whitespace: bool) -> Result<String, Error> {
    if remove_whitespace {
        String::from_utf8(
            s.as_bytes()
                .iter()
                .filter_map(|c| if c.is_ascii_whitespace() { None } else { Some(c.to_owned()) })
                .collect(),
        )
        .map_err(|e| Error::runtime("error removing whitespace").with_cause(Box::new(e)))
    } else if trim {
        Ok(s.trim_ascii().to_string())
    } else {
        Ok(s.to_string())
    }
}

/// Validates the reference (the program output) against the possible
/// alternatives. If alternatives is None, then the check is skipped.
fn validate_alternatives(
    reference: &str,
    alternatives: &[String],
    trim: bool,
    remove_whitespace: bool,
) -> Result<Option<MismatchInfo<String>>, Error> {
    // Empty alternatives means that we ignore this check
    if alternatives.is_empty() {
        return Ok(None);
    };

    let mut found_match = false;
    for alt in alternatives {
        found_match |= treat_output(reference, trim, remove_whitespace)?
            == treat_output(alt, trim, remove_whitespace)?;
        if found_match {
            break;
        }
    }
    if !found_match {
        let mut msgs = vec![];
        if remove_whitespace {
            msgs.push("Whitespaces are ignored.".to_string());
        } else if trim {
            msgs.push("Leading and trailing whitespaces are ignored.".to_string());
        };
        Ok(Some(MismatchInfo {
            received: reference.to_string(),
            allowed_alternatives: alternatives.to_owned(),
            msgs,
        }))
    } else {
        Ok(None)
    }
}

/// Validates the reference code (the exit code of the program) against the
/// allowed alternative. This is the same as for `validate_alternatives`, but
/// for `i32`.
fn validate_alternatives_i32(reference: i32, alternatives: &[i32]) -> Option<MismatchInfo<i32>> {
    // Empty alternatives means that we ignore this check
    if alternatives.is_empty() {
        return None;
    };

    if alternatives.contains(&reference) {
        None
    } else {
        Some(MismatchInfo {
            received: reference,
            allowed_alternatives: alternatives.to_vec(),
            msgs: vec![],
        })
    }
}

/// Generates a template failure report with the basic information present,
/// including the executed command, standard input, and any of the input files.
fn base_report(
    bin: &str,
    cmdargs: &[String],
    infiles: &[container::InputFile],
    stdin: Option<&str>,
) -> Result<DetailsTestFailure, Error> {
    let mut cmdvec = vec![format!("./{bin}")];
    cmdvec.extend_from_slice(cmdargs);

    let mut infile_contents = vec![];
    for (i, infile) in infiles.iter().enumerate() {
        let path = Path::new(&infile.host_path);

        let content = std::fs::File::open(path)
            .and_then(|mut f| {
                let mut buf = String::new();
                f.read_to_string(&mut buf)?;
                Ok(buf)
            })
            .inspect_err(|e| {
                log::error!("Could not read input file when creating error report: {e}")
            })?;

        if infiles.len() > 1 {
            cmdvec.push(format!("INPUT_FILE{}", i + 1));
        } else {
            cmdvec.push("INPUT_FILE".to_string());
        }
        infile_contents.push(SourceFileInfo {
            content,
            extension: path.extension().and_then(|ex| ex.to_str()).map(String::from),
        });
    }
    Ok(DetailsTestFailure {
        command: Some(cmdvec.join(" ")),
        stdin_contents: stdin
            .map(|s| SourceFileInfo { content: s.to_string(), ..Default::default() }),
        input_file_contents: infile_contents,
        ..Default::default()
    })
}

/// Turns a failure to run a command at all into a graded failure. A timeout or
/// a flooded output stream is the student's fault; anything else is ours and
/// propagates. `base` is the failure report to extend, or `None` when no report
/// was asked for. `stage` names the pipeline step for the multi-stage kinds.
fn report_execution_error(
    mut e: Error,
    base: Option<DetailsTestFailure>,
    stage: Option<&str>,
) -> Result<GradingResult, Error> {
    let during = stage.map(|s| format!(" when {s}")).unwrap_or_default();
    match e.kind.as_mut() {
        ErrorKind::Syscommand(SyscommandError {
            timeout: Some(duration), stdout, stderr, ..
        }) => Ok(GradingResult::Failure {
            cause: FailureCause::Timeout(*duration),
            report: base.map(|b| {
                Box::new(DetailsTestFailure {
                    additional_failure_causes: vec![format!(
                        "Timed out after {} seconds{during}.",
                        duration.as_secs(),
                    )],
                    stdout_captured: stdout.take(),
                    stderr_captured: stderr.take(),
                    ..b
                })
            }),
        }),
        ErrorKind::Syscommand(SyscommandError { output_limit_exceeded: Some(limit), .. }) => {
            Ok(GradingResult::Failure {
                cause: FailureCause::OutputLimitExceeded { limit: *limit },
                report: base.map(|b| {
                    Box::new(DetailsTestFailure {
                        additional_failure_causes: vec![format!(
                            "Output stream exceeded {limit} bytes{during}."
                        )],
                        ..b
                    })
                }),
            })
        }
        _ => {
            log::error!("Unknown error happened when running test case in a container: {e}");
            Err(e)
        }
    }
}

#[derive(Debug, Clone)]
pub enum GradingResult {
    Success,
    Failure {
        /// Cause of the failure
        cause: FailureCause,
        /// Provided error report if a base report was passed to `grade()`.
        report: Option<Box<DetailsTestFailure>>,
    },
}

/// What the output of one command is checked against. An empty list of allowed
/// values means that the stream is ignored.
#[derive(Debug, Clone, Default)]
pub struct Expectation<'a> {
    pub code: &'a [i32],
    pub stdout: &'a [String],
    pub stdout_trim: bool,
    pub stdout_rm_whitespace: bool,
    pub stderr: &'a [String],
    pub stderr_trim: bool,
    pub stderr_rm_whitespace: bool,
}

impl Expectation<'_> {
    /// Checks one command's output. `base` builds the report a failure is
    /// described in, and is `None` when no report was asked for. It is only
    /// called when the output does not match.
    fn check<F>(&self, out: &SyscommandOutput, base: Option<F>) -> Result<GradingResult, Error>
    where
        F: FnOnce() -> Result<DetailsTestFailure, Error>,
    {
        let code_mismatch = validate_alternatives_i32(out.code, self.code);
        let stdout_mismatch = validate_alternatives(
            &out.stdout,
            self.stdout,
            self.stdout_trim,
            self.stdout_rm_whitespace,
        )?;
        let stderr_mismatch = validate_alternatives(
            &out.stderr,
            self.stderr,
            self.stderr_trim,
            self.stderr_rm_whitespace,
        )?;

        match (&code_mismatch, &stdout_mismatch, &stderr_mismatch) {
            (None, None, None) => Ok(GradingResult::Success),
            _ => Ok(GradingResult::Failure {
                cause: FailureCause::OutputMismatch,
                report: match base {
                    Some(base) => Some(Box::new(DetailsTestFailure {
                        code_captured: code_mismatch.is_none().then_some(out.code),
                        code_mismatch,
                        stdout_captured: stdout_mismatch.is_none().then(|| out.stdout.to_owned()),
                        stdout_mismatch,
                        stderr_captured: stderr_mismatch.is_none().then(|| out.stderr.to_owned()),
                        stderr_mismatch,
                        ..base()?
                    })),
                    None => None,
                },
            }),
        }
    }
}

/// Grading of a single test case, one function per test kind.
pub mod grade {
    use super::*;

    // .---------------------------------------------------------------------.
    // |  _____         _   _    _           _     _ _ ____             _ _  |
    // | |_   _|__  ___| |_| | _(_)_ __   __| |_  ( | )  _ \ _   _ _ __( | ) |
    // |   | |/ _ \/ __| __| |/ / | '_ \ / _` (_)  V V| |_) | | | | '_ \V V  |
    // |   | |  __/\__ \ |_|   <| | | | | (_| |_      |  _ <| |_| | | | |    |
    // |   |_|\___||___/\__|_|\_\_|_| |_|\__,_(_)     |_| \_\\__,_|_| |_|    |
    // '---------------------------------------------------------------------'

    /// Grades testkind "run". This runs the provided `bin` with arguments and
    /// input files, and validates the output.
    ///
    /// Example:
    ///  - `bin` = `"myprog"`
    ///  - `args` = `["--bar", "foo"]`
    ///  - `input_files` = `["/home/user/test1.txt", "/srv/data/test2.txt"]`
    /// ```sh
    /// ./myprog --bar foo /mnt/testfiles/test1.in /mnt/testfiles/test2.in
    /// ```
    pub fn run(
        kind: &TestkindRun,
        test: &Test,
        container: &ContainerInfo,
        include_report: bool,
    ) -> Result<GradingResult, Error> {
        let infiles = container::InputFile::assign(&kind.input_files, &container.tests_dir)?;
        let executable = format!("./{}", kind.bin);
        let mut cmd: Vec<&str> = vec![&executable];
        cmd.extend(kind.args.iter().map(String::as_str));

        let base = include_report
            .then_some(|| base_report(&kind.bin, &kind.args, &infiles, kind.stdin.as_deref()));

        let out = match container.exec(&container::ExecOptions {
            workdir: &container.internal_build_dir,
            cmd: &cmd,
            infiles: &infiles,
            stdin: kind.stdin.as_deref(),
            expected_code: None,
            max_output: test.max_output,
            timeout: test.timeout,
        }) {
            Ok(out) => out,
            Err(e) => return report_execution_error(e, base.map(|f| f()).transpose()?, None),
        };

        Expectation {
            code: &kind.code,
            stdout: &kind.stdout,
            stdout_trim: kind.stdout_trim,
            stdout_rm_whitespace: kind.stdout_strip_whitespace,
            stderr: &kind.stderr,
            stderr_trim: kind.stderr_trim,
            stderr_rm_whitespace: kind.stderr_strip_whitespace,
        }
        .check(&out, base)
    }

    // .----------------------------------------------------------------.
    // |  _____         _   _    _           _      ____                |
    // | |_   _|__  ___| |_| | _(_)_ __   __| |_   / ___| ___ _ __      |
    // |   | |/ _ \/ __| __| |/ / | '_ \ / _` (_) | |  _ / _ \ '_ \     |
    // |   | |  __/\__ \ |_|   <| | | | | (_| |_  | |_| |  __/ | | |    |
    // |   |_|\___||___/\__|_|\_\_|_| |_|\__,_(_)_ \____|\___|_| |_|    |
    // |    / \  / ___||  \/  |   __ _ _ __   __| | |  _ \ _   _ _ __   |
    // |   / _ \ \___ \| |\/| |  / _` | '_ \ / _` | | |_) | | | | '_ \  |
    // |  / ___ \ ___) | |  | | | (_| | | | | (_| | |  _ <| |_| | | | | |
    // | /_/   \_\____/|_|  |_|  \__,_|_| |_|\__,_| |_| \_\\__,_|_| |_| |
    // '----------------------------------------------------------------'

    /// Grades testkind "gen_asm_and_run". This runs the provided `bin` with
    /// arguments and input files, which should generate some assembly on
    /// standard output. This is then compiled, and the compiled binary is then
    /// graded.
    pub fn gen_asm_and_run(
        settings: &Settings,
        kind: &TestkindGenASMAndRun,
        test: &Test,
        container: &ContainerInfo,
        include_report: bool,
    ) -> Result<GradingResult, Error> {
        const STATUS_ASSEMBLING: &str = "assembling the generated assembly program";
        const STATUS_COMPILING: &str = "compiling the generated assembly program";
        const STATUS_RUNNING: &str = "running the compiled assembly";

        let infiles = container::InputFile::assign(&kind.input_files, &container.tests_dir)?;
        let executable = format!("./{}", kind.bin);
        let mut cmd: Vec<&str> = vec![&executable];
        cmd.extend(kind.args.iter().map(String::as_str));

        let base = || base_report(&kind.bin, &kind.args, &infiles, kind.stdin.as_deref());
        let run_report = include_report.then_some(base);

        // Generate the assembly
        let generated = match container.exec(&container::ExecOptions {
            workdir: &container.internal_build_dir,
            cmd: &cmd,
            infiles: &infiles,
            stdin: kind.stdin.as_deref(),
            expected_code: None,
            max_output: test.max_output,
            timeout: test.timeout,
        }) {
            Ok(out) => out,
            Err(e) => return report_execution_error(e, run_report.map(|f| f()).transpose()?, None),
        };

        match (Expectation {
            code: &kind.code,
            stderr: &kind.stderr,
            stderr_trim: kind.stderr_trim,
            stderr_rm_whitespace: kind.stderr_strip_whitespace,
            ..Default::default()
        })
        .check(&generated, run_report)?
        {
            GradingResult::Success => {}
            fail_res @ GradingResult::Failure { .. } => {
                return Ok(fail_res);
            }
        }
        let generated_assembly = generated.stdout;

        // Every later stage reports the generated assembly alongside.
        let with_asm = || {
            Ok(DetailsTestFailure {
                additional_files: vec![(
                    "Generated Assembly".to_string(),
                    SourceFileInfo {
                        content: generated_assembly.to_owned(),
                        extension: Some("asm".to_string()),
                    },
                )],
                ..base()?
            })
        };
        let asm_report = include_report.then_some(with_asm);

        // Set up the /tmp/grading dir and write the asm program there
        let hostpath_asm = path_absolute_join(&container.tests_dir.host_path, "gen.asm")?;
        let containerpath_asm = path_absolute_join(&container.tests_dir.container_path, "gen.asm")?;

        let stage_exec = container::ExecOptions {
            workdir: GRADING_DIR,
            cmd: &[],
            infiles: &[],
            stdin: None,
            expected_code: None,
            max_output: test.max_output,
            timeout: test.timeout,
        };

        // Open the file in a separate scope to ensure that it is closed
        {
            let mut asm_f = std::fs::File::create(&hostpath_asm)
                .inspect_err(|e| log::error!("Cannot create ASM file {hostpath_asm}: {e}"))?;
            write_all_timeout(
                &mut asm_f,
                generated_assembly.as_bytes(),
                Duration::from_secs(settings.fs_write_timeout_seconds.into()),
            )?;
            asm_f.flush()?;
        }

        // Now write the generated assembly program to a path
        let gradingpath_asm = format!("{GRADING_DIR}/gen.asm");
        let asm_setup = format!(
            "rm -rf {GRADING_DIR} && mkdir -p {GRADING_DIR} \
             && cp \"{containerpath_asm}\" \"{gradingpath_asm}\""
        );
        container.exec(&container::ExecOptions {
            cmd: &["bash", "-c", &asm_setup],
            workdir: "/", // cannot be in GRADING_DIR while setting it up
            expected_code: Some(0),
            ..stage_exec
        })?;
        std::fs::remove_file(&hostpath_asm)
            .inspect_err(|e| log::error!("Error removing ASM file {hostpath_asm}: {e}"))?;

        // Set up the ASM command to  replace the template <ASM_FILE> with the
        // true filename.
        let asm_cmd: Vec<&str> = kind
            .assemble_cmd
            .iter()
            .map(|s| if s == "<ASM_FILE>" { gradingpath_asm.as_str() } else { s.as_str() })
            .collect();

        let compile_cmd: Vec<&str> = kind.compile_cmd.iter().map(String::as_str).collect();

        // Assemble and then compile
        for (exec_cmd, allowed_codes, stat_msg) in [
            (&asm_cmd, &kind.assemble_code, STATUS_ASSEMBLING),
            (&compile_cmd, &kind.compile_code, STATUS_COMPILING),
        ] {
            let out = match container.exec(&container::ExecOptions { cmd: exec_cmd, ..stage_exec })
            {
                Ok(out) => out,
                Err(e) => {
                    return report_execution_error(
                        e,
                        asm_report.map(|f| f()).transpose()?,
                        Some(stat_msg),
                    )
                }
            };

            match (Expectation { code: allowed_codes, ..Default::default() }).check(
                &out,
                include_report.then_some(|| {
                    Ok(DetailsTestFailure {
                        additional_failure_causes: vec![format!("Error when {stat_msg}.")],
                        ..with_asm()?
                    })
                }),
            )? {
                GradingResult::Success => {} // ok
                fail_res @ GradingResult::Failure { .. } => {
                    return Ok(fail_res);
                }
            }
        }

        // Finally run the compiled binary and check the output
        let run_cmd: Vec<&str> = kind.run_cmd.iter().map(String::as_str).collect();

        let out = match container.exec(&container::ExecOptions {
            cmd: &run_cmd,
            stdin: kind.run_stdin.as_deref(),
            ..stage_exec
        }) {
            Ok(out) => out,
            Err(e) => {
                return report_execution_error(
                    e,
                    asm_report.map(|f| f()).transpose()?,
                    Some(STATUS_RUNNING),
                )
            }
        };

        Expectation {
            code: &kind.run_code,
            stdout: &kind.run_stdout,
            stdout_trim: kind.run_stdout_trim,
            stdout_rm_whitespace: kind.run_stdout_strip_whitespace,
            stderr: &kind.run_stderr,
            stderr_trim: kind.run_stderr_trim,
            stderr_rm_whitespace: kind.run_stderr_strip_whitespace,
        }
        .check(&out, asm_report)
    }

    // .-----------------------------------------------------------------------.
    // |  _____         _   _    _           _      ____ _               _     |
    // | |_   _|__  ___| |_| | _(_)_ __   __| |_   / ___| |__   ___  ___| | __ |
    // |   | |/ _ \/ __| __| |/ / | '_ \ / _` (_) | |   | '_ \ / _ \/ __| |/ / |
    // |   | |  __/\__ \ |_|   <| | | | | (_| |_  | |___| | | |  __/ (__|   <  |
    // |   |_|\___||___/\__|_|\_\_|_| |_|\__,_(_)  \____|_| |_|\___|\___|_|\_\ |
    // |  _____ _ _        _____      _     _                                  |
    // | |  ___(_) | ___  | ____|_  _(_)___| |_ ___                            |
    // | | |_  | | |/ _ \ |  _| \ \/ / / __| __/ __|                           |
    // | |  _| | | |  __/ | |___ >  <| \__ \ |_\__ \                           |
    // | |_|   |_|_|\___| |_____/_/\_\_|___/\__|___/                           |
    // '-----------------------------------------------------------------------'

    /// Grades testkind "check_file_exists". Just checks that the specified file
    /// exists, and optionally some properties of that file.
    pub fn check_file_exists(
        kind: &TestkindCheckFileExists,
        container: &ContainerInfo,
        include_report: bool,
    ) -> Result<GradingResult, Error> {
        let check_path = path_absolute_join(&container.solution_dir.host_path, &kind.path)?;

        if !std::fs::exists(&check_path)? {
            return Ok(GradingResult::Failure {
                cause: FailureCause::OutputMismatch,
                report: include_report.then(|| {
                    Box::new(DetailsTestFailure {
                        additional_failure_causes: vec!["File not found.".to_string()],
                        checked_files: vec![kind.path.to_owned()],
                        ..Default::default()
                    })
                }),
            });
        }

        if let Some(check_prefix) = kind.mimetype_prefix.as_deref() {
            let ident_mimetype = utils::mimetype(&check_path)
                .inspect_err(|e| log::error!("Could not check file {check_path}: {e}"))?;
            if !ident_mimetype.starts_with(check_prefix) {
                return Ok(GradingResult::Failure {
                    cause: FailureCause::OutputMismatch,
                    report: include_report.then(|| {
                        Box::new(DetailsTestFailure {
                            additional_failure_causes: vec!["Invalid MIME-type.".to_string()],
                            mimetype_mismatch_files: vec![MIMETypeInfo {
                                path: kind.path.to_owned(),
                                mime_identified: ident_mimetype,
                                mime_expected: Some(check_prefix.to_string()),
                            }],
                            ..Default::default()
                        })
                    }),
                });
            }
        }

        Ok(GradingResult::Success)
    }

    // .--------------------------------------------------------------.
    // |  _____         _   _    _           _     ____               |
    // | |_   _|__  ___| |_| | _(_)_ __   __| |_  |  _ \ _   _ _ __   |
    // |   | |/ _ \/ __| __| |/ / | '_ \ / _` (_) | |_) | | | | '_ \  |
    // |   | |  __/\__ \ |_|   <| | | | | (_| |_  |  _ <| |_| | | | | |
    // | __|_|\___||___/\__|_|\_\_|_| |_|\__,_(_) |_| \_\\__,_|_| |_| |
    // | \ \   / /__ _ __(_)/ _(_) ___ _ __                           |
    // |  \ \ / / _ \ '__| | |_| |/ _ \ '__|                          |
    // |   \ V /  __/ |  | |  _| |  __/ |                             |
    // |    \_/ \___|_|  |_|_| |_|\___|_|                             |
    // '--------------------------------------------------------------'

    /// Grades testkind "run_verifier". Runs `bin` exactly like the "run" kind
    /// does, then hands the result to a course-provided verifier program, which
    /// is what decides pass or fail. `script` is the path of that program
    /// inside the verifier container.
    pub fn run_verifier(
        kind: &TestkindRunVerifier,
        test: &Test,
        container: &ContainerInfo,
        verifier: &verifier::Verifier,
        script: &str,
        include_report: bool,
    ) -> Result<GradingResult, Error> {
        let infiles = container::InputFile::assign(&kind.input_files, &container.tests_dir)?;
        let executable = format!("./{}", kind.bin);
        let mut cmd: Vec<&str> = vec![&executable];
        cmd.extend(kind.args.iter().map(String::as_str));

        let base = include_report
            .then_some(|| base_report(&kind.bin, &kind.args, &infiles, kind.stdin.as_deref()));

        let exec_opts = container::ExecOptions {
            workdir: &container.internal_build_dir,
            cmd: &cmd,
            infiles: &infiles,
            stdin: kind.stdin.as_deref(),
            expected_code: None,
            max_output: test.max_output,
            timeout: test.timeout,
        };
        let output = match container.exec(&exec_opts) {
            Ok(output) => output,
            // A solution that timed out or flooded a stream never reaches the
            // verifier: there is no complete output to judge.
            Err(e) => return report_execution_error(e, base.map(|f| f()).transpose()?, None),
        };

        let mut files = BTreeMap::new();
        for infile in &infiles {
            let bytes = std::fs::read(&infile.host_path).map_err(|e| {
                Error::fs("reading input file for verifier", &infile.host_path)
                    .with_cause(Box::new(e))
            })?;
            files.insert(infile.name.to_owned(), verifier::Encoded::new(&bytes));
        }

        let verdict = verifier.verify(
            script,
            &verifier::VerifierInput {
                cmd: &exec_opts.exec_cmd(),
                code: output.code,
                stdout: verifier::Encoded::Utf8(output.stdout.clone()),
                stderr: verifier::Encoded::Utf8(output.stderr.clone()),
                files,
                params: &kind.verifier_params,
            },
            kind.verifier_timeout,
        )?;

        if verdict.accepted {
            return Ok(GradingResult::Success);
        }

        Ok(GradingResult::Failure {
            cause: FailureCause::OutputMismatch,
            report: match base.map(|f| f()).transpose()? {
                Some(b) => Some(Box::new(DetailsTestFailure {
                    additional_failure_causes: vec![verdict
                        .reason
                        .unwrap_or_else(|| "Rejected by the verifier.".to_string())],
                    code_captured: Some(output.code),
                    stdout_captured: Some(output.stdout),
                    stderr_captured: Some(output.stderr),
                    ..b
                })),
                None => None,
            },
        })
    }
}
