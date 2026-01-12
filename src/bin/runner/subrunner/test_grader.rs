/// This file contains the functionality used to grade a test case.
use std::{io::Read, io::Write, path::Path, time::Duration};

use id2202_autograder::{
    config::{
        tests::{TestkindCheckFileExists, TestkindGenASMAndRun, TestkindRun},
        TestDefault,
    },
    error::{Error, ErrorKind, SyscommandError},
    reporting::{DetailsTestFailure, MIMETypeInfo, MismatchInfo, SourceFileInfo},
    utils::{self, path_absolute_join, syscommand_timeout, SyscommandSettings},
};

use crate::subrunner::container::ContainerInfo;

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
                .filter_map(|c| {
                    if c.is_ascii_whitespace() {
                        None
                    } else {
                        Some(c.to_owned())
                    }
                })
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
            == treat_output(&alt, trim, remove_whitespace)?;
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
            msgs: msgs,
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

    if alternatives.iter().any(|c| reference == *c) {
        None
    } else {
        Some(MismatchInfo {
            received: reference,
            allowed_alternatives: alternatives.to_vec(),
            msgs: vec![],
        })
    }
}

#[derive(Debug, Clone)]
pub enum GradingResult {
    Success {
        /// This is the captured stdout from the run binary. It will only be
        /// populated if the run configuration explicitly states that standard
        /// output should be captured. This is typically when output needs to
        /// be forwarded to a later stage.
        captured_stdout: String,
    },
    Failure {
        /// Cause of the failure
        cause: FailureCause,
        /// Provided error report if `include_report` is set to `true` when calling `grade()`.
        report: Option<Box<DetailsTestFailure>>,
    },
}

// .---------------------------------------------------------------------.
// |  _____         _   _    _           _     _ _ ____             _ _  |
// | |_   _|__  ___| |_| | _(_)_ __   __| |_  ( | )  _ \ _   _ _ __( | ) |
// |   | |/ _ \/ __| __| |/ / | '_ \ / _` (_)  V V| |_) | | | | '_ \V V  |
// |   | |  __/\__ \ |_|   <| | | | | (_| |_      |  _ <| |_| | | | |    |
// |   |_|\___||___/\__|_|\_\_|_| |_|\__,_(_)     |_| \_\\__,_|_| |_|    |
// '---------------------------------------------------------------------'

/// Configuration for testkind "run". This runs the provided `bin` with
/// arguments and input files, and validates the output.
///
/// Example:
///  - `bin` = `"myprog"`
///  - `cmdargs` = `["--bar", "foo"]`
///  - `infile_paths` = `["/home/user/test1.txt", "/srv/data/test2.txt"]`
/// ```sh
/// ./myprog --bar foo /mnt/testfiles/test1.in /mnt/testfiles/test2.in
/// ```
#[derive(Debug, Clone)]
pub struct Run<'a> {
    /// Information about the container to run inside
    pub container: &'a ContainerInfo,
    /// Name of the binary to run
    pub bin: &'a str,
    /// Arguments to pass to the binary
    pub cmdargs: &'a [String],
    /// Paths to files to provide as input. The paths should be as seen from
    /// outside the container.
    pub infile_paths: &'a [String],
    /// Text to pass through standard input. `None` means that nothing is
    /// passed to standard input.
    pub stdin: Option<&'a str>,
    /// Whether or not we should capture standard output
    pub capture_stdout: bool,
    /// Maximum output in bytes
    pub max_output: usize,
    /// Timeout limit in seconds
    pub timeout: u32,
    /// Allowed return codes
    pub code_allowed_values: &'a [i32],
    /// Allowed standard output values. An empty vector means that stdout is ignored
    pub stdout_allowed_values: &'a [String],
    pub stdout_trim: bool,
    pub stdout_rm_whitespace: bool,
    /// Allowed standard error values. An empty vector means that stdout is ignored
    pub stderr_allowed_values: &'a [String],
    pub stderr_trim: bool,
    pub stderr_rm_whitespace: bool,
}

impl<'a> Run<'a> {
    /// Instantiate this test case from a testkind and then grade it.
    pub fn grade_from_testkind(
        kind: &TestkindRun,
        test_default: &TestDefault,
        container: &ContainerInfo,
        include_report: bool,
    ) -> Result<GradingResult, Error> {
        Run {
            container: container,
            bin: &kind.bin,
            cmdargs: &kind.args,
            infile_paths: &kind.input_files,
            stdin: if kind.stdin_ignore {
                None
            } else {
                Some(kind.stdin.as_str())
            },
            capture_stdout: false,
            max_output: test_default.max_output,
            timeout: test_default.timeout_test,
            code_allowed_values: &kind.code,
            stdout_allowed_values: &kind.stdout,
            stdout_trim: kind.stdout_trim,
            stdout_rm_whitespace: kind.stdout_strip_whitespace,
            stderr_allowed_values: &kind.stderr,
            stderr_trim: kind.stderr_trim,
            stderr_rm_whitespace: kind.stderr_strip_whitespace,
        }
        .grade(include_report)
    }

    /// Common function to run the compiled solution for different test kinds,
    /// performing checks such that the output is what would be expected as
    /// well.
    pub fn grade(&self, include_report: bool) -> Result<GradingResult, Error> {
        // list of files to clean up on the host system after the container has
        // finished running
        let mut hostfiles_to_remove = vec![];

        let executable = format!("./{}", self.bin);
        let mut test_cmd: Vec<String> = vec![
            "podman".into(),
            "exec".into(),
            "-w".into(),
            self.container.internal_build_dir.clone(),
        ];
        if self.stdin.is_some() {
            // This is needed for podman to capture stdin
            test_cmd.push("-i".into());
        }
        test_cmd.push(self.container.podman_container_name.clone());
        test_cmd.push(executable.clone());
        test_cmd.extend_from_slice(self.cmdargs);
        for (i, infile) in self.infile_paths.iter().enumerate() {
            let hostfile =
                path_absolute_join(&self.container.external_tests, format!("test{i}.in"))?;
            let containerfile =
                path_absolute_join(&self.container.mount_tests, format!("test{i}.in"))?;
            // Copy file to the external_tests dirs and add the internal file
            // path to the command
            std::fs::copy(infile, &hostfile).inspect_err(|e| {
                log::error!("Could not copy input file {} to {}: {e}", infile, hostfile)
            })?;
            test_cmd.push(containerfile);
            hostfiles_to_remove.push(hostfile);
        }

        let res = syscommand_timeout(
            test_cmd.as_slice(),
            SyscommandSettings {
                stdin: self.stdin.map(String::from),
                max_stdout_length: Some(self.max_output),
                max_stderr_length: Some(self.max_output),
                timeout: Duration::from_secs(self.timeout.into()),
                ..Default::default()
            },
        );
        for fpath in hostfiles_to_remove {
            // Remove the file that was used in the test case
            std::fs::remove_file(&fpath)
                .unwrap_or_else(|e| log::error!("Could not remove input file \"{fpath}\": {e}"));
        }

        match res {
            Ok(output) => {
                // Check the expected statuses (if we are checking the return code)
                let code_mismatch =
                    validate_alternatives_i32(output.code, self.code_allowed_values);
                let stdout_mismatch = validate_alternatives(
                    &output.stdout,
                    self.stdout_allowed_values,
                    self.stdout_trim,
                    self.stdout_rm_whitespace,
                )?;
                let stderr_mismatch = validate_alternatives(
                    &output.stderr,
                    self.stderr_allowed_values,
                    self.stderr_trim,
                    self.stderr_rm_whitespace,
                )?;

                match (&code_mismatch, &stdout_mismatch, &stderr_mismatch) {
                    (None, None, None) => {
                        return Ok(GradingResult::Success {
                            captured_stdout: if self.capture_stdout {
                                output.stdout
                            } else {
                                "".to_string()
                            },
                        });
                    }
                    _ => {
                        let report = if include_report {
                            Some(Box::new(DetailsTestFailure {
                                code_captured: if code_mismatch.is_none() {
                                    Some(output.code)
                                } else {
                                    None
                                },
                                code_mismatch: code_mismatch,
                                stdout_captured: if stdout_mismatch.is_none() {
                                    Some(output.stdout)
                                } else {
                                    None
                                },
                                stdout_mismatch: stdout_mismatch,
                                stderr_captured: if stderr_mismatch.is_none() {
                                    Some(output.stderr)
                                } else {
                                    None
                                },
                                stderr_mismatch: stderr_mismatch,
                                ..self.base_report()?
                            }))
                        } else {
                            None
                        };

                        return Ok(GradingResult::Failure {
                            cause: FailureCause::OutputMismatch,
                            report: report,
                        });
                    }
                }
            }
            Err(Error {
                kind:
                    ErrorKind::Syscommand(SyscommandError {
                        timeout: Some(duration),
                        stdout,
                        stderr,
                        ..
                    }),
                ..
            }) => {
                return Ok(GradingResult::Failure {
                    cause: FailureCause::Timeout(duration),
                    report: if include_report {
                        Some(Box::new(DetailsTestFailure {
                            additional_failure_causes: vec![format!(
                                "Timed out after {} seconds.",
                                duration.as_secs(),
                            )],
                            stdout_captured: stdout,
                            stderr_captured: stderr,
                            ..self.base_report()?
                        }))
                    } else {
                        None
                    },
                });
            }
            Err(Error {
                kind:
                    ErrorKind::Syscommand(SyscommandError {
                        output_limit_exceeded: Some(limit),
                        ..
                    }),
                ..
            }) => {
                return Ok(GradingResult::Failure {
                    cause: FailureCause::OutputLimitExceeded { limit: limit },
                    report: if include_report {
                        Some(Box::new(DetailsTestFailure {
                            additional_failure_causes: vec![format!(
                                "Output stream exceeded {} bytes.",
                                limit
                            )],
                            ..self.base_report()?
                        }))
                    } else {
                        None
                    },
                });
            }
            Err(e) => {
                log::error!("Unknown error happened when running test case in a container: {e}");
                return Err(e);
            }
        }
    }

    /// Generates a template failure report with the basic information present,
    /// including the executed command, standard input, and any of the input
    /// files.
    fn base_report(&self) -> Result<DetailsTestFailure, Error> {
        let mut cmdvec = vec![format!("./{}", self.bin)];
        cmdvec.extend_from_slice(self.cmdargs);

        let mut infile_contents = vec![];
        for (i, strpath) in self.infile_paths.iter().enumerate() {
            let path = Path::new(strpath);

            let content = std::fs::File::open(path)
                .and_then(|mut f| {
                    let mut buf = String::new();
                    f.read_to_string(&mut buf)?;
                    Ok(buf)
                })
                .inspect_err(|e| {
                    log::error!("Could not read input file when creating error report: {e}")
                })?;

            if self.infile_paths.len() > 1 {
                cmdvec.push(format!("INPUT_FILE{}", i + 1));
            } else {
                cmdvec.push("INPUT_FILE".to_string());
            }
            infile_contents.push(SourceFileInfo {
                content: content,
                extension: path
                    .extension()
                    .and_then(|ex| ex.to_str())
                    .map(String::from),
            });
        }
        Ok(DetailsTestFailure {
            command: Some(cmdvec.join(" ")),
            stdin_contents: self.stdin.map(|s| SourceFileInfo {
                content: s.to_string(),
                ..Default::default()
            }),
            input_file_contents: infile_contents,
            ..Default::default()
        })
    }
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

/// Configuration for testkind "gen_asm_and_run". This runs the provided `bin`
/// with arguments and input files, which should generate some assembly on
/// standard output. This is then compiled, and the compiled binary is then
/// graded.
#[derive(Debug, Clone)]
pub struct GenASMAndRun<'a> {
    /// Information about the container to run inside
    pub container: &'a ContainerInfo,
    /// Name of the binary to run
    pub bin: &'a str,
    /// Arguments to pass to the binary
    pub cmdargs: &'a [String],
    /// Paths to files to provide as input. The paths should be as seen from
    /// outside the container.
    pub infile_paths: &'a [String],
    /// Text to pass through standard input. `None` means that nothing is
    /// passed to standard input.
    pub stdin: Option<&'a str>,
    /// Maximum output in bytes
    pub max_output: usize,
    /// Timeout limit in seconds
    pub timeout: u32,
    /// Allowed return codes when running binary generating the assembly code.
    pub code_allowed_values: &'a [i32],
    /// Allowed standard error values when running binary generating the
    /// assembly code. An empty list means that stderr is ignored.
    pub stderr_allowed_values: &'a [String],
    pub stderr_trim: bool,
    pub stderr_rm_whitespace: bool,

    /// Command to run the assembler with
    pub assemble_cmd: &'a [String],
    /// Allowed return codes when running the assembler
    pub assemble_code_allowed_values: &'a [i32],

    /// Command to run the compiler with
    pub compile_cmd: &'a [String],
    /// Allowed return codes when running the compiler
    pub compile_code_allowed_values: &'a [i32],

    /// Command to run the compiled binary with
    pub run_cmd: &'a [String],
    /// Text to pass through standard input when running the compiled binary.
    /// `None` means that nothing is passed to standard input.
    pub run_stdin: Option<&'a str>,
    /// Allowed return codes when running the compiled binary
    pub run_code_allowed_values: &'a [i32],
    /// Allowed standard output values when running the compiled binary. An
    /// empty list means that stdout is ignored.
    pub run_stdout_allowed_values: &'a [String],
    pub run_stdout_trim: bool,
    pub run_stdout_rm_whitespace: bool,
    /// Allowed standard error values when running the compiled binary. An
    /// empty list means that stderr is ignored.
    pub run_stderr_allowed_values: &'a [String],
    pub run_stderr_trim: bool,
    pub run_stderr_rm_whitespace: bool,
}

impl<'a> GenASMAndRun<'a> {
    /// The input used when piggy-backing on the existing run-grader.
    fn run_input(&self) -> Run<'a> {
        Run {
            container: self.container,
            bin: self.bin,
            cmdargs: self.cmdargs,
            infile_paths: self.infile_paths,
            stdin: self.stdin,
            capture_stdout: true,
            max_output: self.max_output,
            timeout: self.timeout,
            code_allowed_values: self.code_allowed_values,
            stdout_allowed_values: &[],
            stdout_trim: false,
            stdout_rm_whitespace: false,
            stderr_allowed_values: self.stderr_allowed_values,
            stderr_trim: self.stderr_trim,
            stderr_rm_whitespace: self.stderr_rm_whitespace,
        }
    }

    /// Instantiate this test case from a testkind and then grade it.
    pub fn grade_from_testkind(
        kind: &TestkindGenASMAndRun,
        test_default: &TestDefault,
        container: &ContainerInfo,
        include_report: bool,
    ) -> Result<GradingResult, Error> {
        GenASMAndRun {
            container: container,
            bin: &kind.bin,
            cmdargs: &kind.args,
            infile_paths: &kind.input_files,
            stdin: if kind.stdin_ignore {
                None
            } else {
                Some(kind.stdin.as_str())
            },
            max_output: test_default.max_output,
            timeout: test_default.timeout_test,
            code_allowed_values: &kind.code,
            stderr_allowed_values: &kind.stderr,
            stderr_trim: kind.stderr_trim,
            stderr_rm_whitespace: kind.stderr_strip_whitespace,
            assemble_cmd: &kind.assemble_cmd,
            assemble_code_allowed_values: &kind.assemble_code,
            compile_cmd: &kind.compile_cmd,
            compile_code_allowed_values: &kind.compile_code,
            run_cmd: &kind.run_cmd,
            run_stdin: if kind.run_stdin_ignore {
                None
            } else {
                Some(kind.run_stdin.as_str())
            },
            run_code_allowed_values: &kind.run_code,
            run_stdout_allowed_values: &kind.run_stdout,
            run_stdout_trim: kind.run_stdout_trim,
            run_stdout_rm_whitespace: kind.run_stdout_strip_whitespace,
            run_stderr_allowed_values: &kind.run_stderr,
            run_stderr_trim: kind.run_stderr_trim,
            run_stderr_rm_whitespace: kind.run_stderr_strip_whitespace,
        }
        .grade(include_report)
    }

    /// Common function to run the compiled solution for different test kinds,
    /// performing checks such that the output is what would be expected as
    /// well.
    pub fn grade(&self, include_report: bool) -> Result<GradingResult, Error> {
        // Reuse the run infrastructure to generate the output assembly.
        let generated_assembly = match self.run_input().grade(include_report)? {
            GradingResult::Success { captured_stdout } => captured_stdout,
            fail_res @ GradingResult::Failure { .. } => {
                return Ok(fail_res);
            }
        };

        // Set up the /tmp/grading dir and write the asm program there
        let hostpath_asm = path_absolute_join(&self.container.external_tests, "gen.asm")?;
        let containerpath_asm = path_absolute_join(&self.container.mount_tests, "gen.asm")?;

        // Open the file in a separate scope to ensure that it is closed
        {
            let mut asm_f = std::fs::File::create(&hostpath_asm)
                .inspect_err(|e| log::error!("Cannot create ASM file {hostpath_asm}: {e}"))?;
            asm_f.write(generated_assembly.as_bytes())?;
            asm_f.flush()?;
        }

        syscommand_timeout(
            &[
                "podman",
                "exec",
                &self.container.podman_container_name,
                "bash",
                "-c",
                &format!("rm -rf /tmp/grading && mkdir -p /tmp/grading && cp \"{containerpath_asm}\" /tmp/grading/gen.asm"),
            ],
            SyscommandSettings {
                expected_code: Some(0),
                ..Default::default()
            },
        )?;
        std::fs::remove_file(&hostpath_asm)
            .inspect_err(|e| log::error!("Error removing ASM file {hostpath_asm}: {e}"))?;

        // Now write the generated assembly program to a path
        // (set up the ASM command separately to make sure that we
        // replace the template <ASM_FILE> with the true filename.)
        let mut asm_cmd: Vec<&str> = vec![
            "podman",
            "exec",
            "-w",
            "/tmp/grading",
            &self.container.podman_container_name,
        ];
        asm_cmd.extend(self.assemble_cmd.iter().map(|s| {
            if s == "<ASM_FILE>" {
                "/tmp/grading/gen.asm"
            } else {
                s.as_str()
            }
        }));

        match self.intermediate_grading(
            include_report,
            &asm_cmd,
            self.assemble_code_allowed_values,
            &generated_assembly,
            "assembling the generated assembly program",
        )? {
            GradingResult::Success { .. } => {} // ok
            fail_res @ GradingResult::Failure { .. } => {
                return Ok(fail_res);
            }
        }

        // Now run the compilation step
        let mut compile_cmd: Vec<&str> = vec![
            "podman",
            "exec",
            "-w",
            "/tmp/grading",
            &self.container.podman_container_name,
        ];
        compile_cmd.extend(self.compile_cmd.iter().map(String::as_str));

        match self.intermediate_grading(
            include_report,
            &compile_cmd,
            self.compile_code_allowed_values,
            &generated_assembly,
            "compiling the generated assembly program",
        )? {
            GradingResult::Success { .. } => {} // ok
            fail_res @ GradingResult::Failure { .. } => {
                return Ok(fail_res);
            }
        }

        // Finally run the compiled binary and check the output
        let mut run_cmd: Vec<&str> = vec!["podman", "exec", "-w", "/tmp/grading"];
        if self.run_stdin.is_some() {
            run_cmd.push("-i");
        }
        run_cmd.push(&self.container.podman_container_name);
        run_cmd.extend(self.run_cmd.iter().map(String::as_str));

        match syscommand_timeout(
            run_cmd.as_slice(),
            SyscommandSettings {
                stdin: self.run_stdin.map(String::from),
                max_stdout_length: Some(self.max_output),
                max_stderr_length: Some(self.max_output),
                timeout: Duration::from_secs(self.timeout.into()),
                ..Default::default()
            },
        ) {
            Ok(output) => {
                // Check the expected statuses (if we are checking the return code)
                let code_mismatch =
                    validate_alternatives_i32(output.code, self.run_code_allowed_values);
                let stdout_mismatch = validate_alternatives(
                    &output.stdout,
                    self.run_stdout_allowed_values,
                    self.run_stdout_trim,
                    self.run_stdout_rm_whitespace,
                )?;
                let stderr_mismatch = validate_alternatives(
                    &output.stderr,
                    self.run_stderr_allowed_values,
                    self.run_stderr_trim,
                    self.run_stderr_rm_whitespace,
                )?;

                match (&code_mismatch, &stdout_mismatch, &stderr_mismatch) {
                    (None, None, None) => {
                        return Ok(GradingResult::Success {
                            captured_stdout: "".to_string(),
                        });
                    }
                    _ => {
                        let report = if include_report {
                            Some(Box::new(DetailsTestFailure {
                                code_captured: if code_mismatch.is_none() {
                                    Some(output.code)
                                } else {
                                    None
                                },
                                code_mismatch: code_mismatch,
                                stdout_captured: if stdout_mismatch.is_none() {
                                    Some(output.stdout)
                                } else {
                                    None
                                },
                                stdout_mismatch: stdout_mismatch,
                                stderr_captured: if stderr_mismatch.is_none() {
                                    Some(output.stderr)
                                } else {
                                    None
                                },
                                stderr_mismatch: stderr_mismatch,
                                ..self.base_report(&generated_assembly)?
                            }))
                        } else {
                            None
                        };

                        return Ok(GradingResult::Failure {
                            cause: FailureCause::OutputMismatch,
                            report: report,
                        });
                    }
                }
            }
            Err(Error {
                kind:
                    ErrorKind::Syscommand(SyscommandError {
                        timeout: Some(duration),
                        stdout,
                        stderr,
                        ..
                    }),
                ..
            }) => {
                return Ok(GradingResult::Failure {
                    cause: FailureCause::Timeout(duration),
                    report: if include_report {
                        Some(Box::new(DetailsTestFailure {
                            additional_failure_causes: vec![format!(
                                "Timed out after {} seconds when running the compiled assembly.",
                                duration.as_secs(),
                            )],
                            stdout_captured: stdout,
                            stderr_captured: stderr,
                            ..self.base_report(&generated_assembly)?
                        }))
                    } else {
                        None
                    },
                });
            }
            Err(Error {
                kind:
                    ErrorKind::Syscommand(SyscommandError {
                        output_limit_exceeded: Some(limit),
                        ..
                    }),
                ..
            }) => {
                return Ok(GradingResult::Failure {
                    cause: FailureCause::OutputLimitExceeded { limit: limit },
                    report: if include_report {
                        Some(Box::new(DetailsTestFailure {
                            additional_failure_causes: vec![format!(
                                "Output stream exceeded {} bytes when running the compiled assembly.",
                                limit
                            )],
                            ..self.base_report(&generated_assembly)?
                        }))
                    } else {
                        None
                    },
                });
            }
            Err(e) => {
                log::error!("Unknown error happened when running test case in a container: {e}");
                return Err(e);
            }
        }
    }

    /// Grading of the intermediate assemble and compile steps. This just runs
    /// an intermediate step command, checking that the output result codes
    /// match what is to be expected.
    fn intermediate_grading(
        &self,
        include_report: bool,
        cmd: &[&str],
        allowed_codes: &[i32],
        generated_assembly: &str,
        stage_description: &str,
    ) -> Result<GradingResult, Error> {
        match syscommand_timeout(
            cmd,
            SyscommandSettings {
                expected_code: None,
                max_stdout_length: Some(self.max_output),
                max_stderr_length: Some(self.max_output),
                timeout: Duration::from_secs(self.timeout.into()),
                ..Default::default()
            },
        ) {
            Ok(output) => {
                if let Some(mm) = validate_alternatives_i32(output.code, allowed_codes) {
                    Ok(GradingResult::Failure {
                        cause: FailureCause::OutputMismatch,
                        report: if include_report {
                            Some(Box::new(DetailsTestFailure {
                                code_mismatch: Some(mm),
                                additional_failure_causes: vec![format!(
                                    "Error when {}.",
                                    stage_description
                                )],
                                stdout_captured: Some(output.stdout),
                                stderr_captured: Some(output.stderr),
                                ..self.base_report(&generated_assembly)?
                            }))
                        } else {
                            None
                        },
                    })
                } else {
                    Ok(GradingResult::Success {
                        captured_stdout: "".to_string(),
                    })
                }
            }
            Err(Error {
                kind:
                    ErrorKind::Syscommand(SyscommandError {
                        timeout: Some(duration),
                        stdout,
                        stderr,
                        ..
                    }),
                ..
            }) => Ok(GradingResult::Failure {
                cause: FailureCause::Timeout(duration),
                report: if include_report {
                    Some(Box::new(DetailsTestFailure {
                        additional_failure_causes: vec![format!(
                            "Timed out after {} seconds when {}.",
                            duration.as_secs(),
                            stage_description,
                        )],
                        stdout_captured: stdout,
                        stderr_captured: stderr,
                        ..self.base_report(&generated_assembly)?
                    }))
                } else {
                    None
                },
            }),
            Err(Error {
                kind:
                    ErrorKind::Syscommand(SyscommandError {
                        output_limit_exceeded: Some(limit),
                        ..
                    }),
                ..
            }) => Ok(GradingResult::Failure {
                cause: FailureCause::OutputLimitExceeded { limit: limit },
                report: if include_report {
                    Some(Box::new(DetailsTestFailure {
                        additional_failure_causes: vec![format!(
                            "Output stream exceeded {} bytes when {}.",
                            limit, stage_description,
                        )],
                        ..self.base_report(&generated_assembly)?
                    }))
                } else {
                    None
                },
            }),
            Err(e) => {
                log::error!("Unknown error happened when running test case in a container: {e}");
                Err(e)
            }
        }
    }

    /// Generates a template failure report with the basic information present,
    /// including the executed command, standard input, and any of the input
    /// files.
    fn base_report(&self, generated_assembly: &str) -> Result<DetailsTestFailure, Error> {
        Ok(DetailsTestFailure {
            additional_files: vec![(
                "Generated Assembly".to_string(),
                SourceFileInfo {
                    content: generated_assembly.to_string(),
                    extension: Some("asm".to_string()),
                },
            )],
            ..self.run_input().base_report()?
        })
    }
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

/// Configuration for testkind "check_file_exists". Just checks that the
/// specified file exists, and optionally some properties of that file.
pub struct CheckFileExists<'a> {
    /// Information about the container to run inside
    pub container: &'a ContainerInfo,
    /// The path inside the solution directory where the file can be found
    pub path: &'a str,
    /// The mimetype that the file should have
    pub mimetype_prefix: Option<&'a str>,
}

impl<'a> CheckFileExists<'a> {
    /// Instantiate this test case from a testkind and then grade it.
    pub fn grade_from_testkind(
        kind: &TestkindCheckFileExists,
        _test_default: &TestDefault,
        container: &ContainerInfo,
        include_report: bool,
    ) -> Result<GradingResult, Error> {
        CheckFileExists {
            container: container,
            path: &kind.path,
            mimetype_prefix: if kind.mimetype_prefix_ignore {
                None
            } else {
                Some(&kind.mimetype_prefix)
            },
        }
        .grade(include_report)
    }

    /// Grades this test case.
    pub fn grade(&self, include_report: bool) -> Result<GradingResult, Error> {
        let check_path = path_absolute_join(&self.container.external_solution, &self.path)?;

        if !std::fs::exists(&check_path)? {
            return Ok(GradingResult::Failure {
                cause: FailureCause::OutputMismatch,
                report: if include_report {
                    Some(Box::new(DetailsTestFailure {
                        additional_failure_causes: vec!["File not found.".to_string()],
                        ..self.base_report()?
                    }))
                } else {
                    None
                },
            });
        }

        if let Some(check_prefix) = self.mimetype_prefix {
            let ident_mimetype = utils::mimetype(&check_path)
                .inspect_err(|e| log::error!("Could not check file {check_path}: {e}"))?;
            if !ident_mimetype.starts_with(&check_prefix) {
                return Ok(GradingResult::Failure {
                    cause: FailureCause::OutputMismatch,
                    report: if include_report {
                        Some(Box::new(DetailsTestFailure {
                            additional_failure_causes: vec!["Invalid MIME-type.".to_string()],
                            checked_files: vec![], // resetting this to not duplicate info
                            mimetype_mismatch_files: vec![MIMETypeInfo {
                                path: self.path.to_string(),
                                mime_identified: ident_mimetype,
                                mime_expected: Some(check_prefix.to_string()),
                            }],
                            ..self.base_report()?
                        }))
                    } else {
                        None
                    },
                });
            }
        }

        Ok(GradingResult::Success {
            captured_stdout: "".to_string(),
        })
    }

    /// Generates a template failure report with the basic information present.
    fn base_report(&self) -> Result<DetailsTestFailure, Error> {
        Ok(DetailsTestFailure {
            checked_files: vec![self.path.to_string()],
            ..Default::default()
        })
    }
}
