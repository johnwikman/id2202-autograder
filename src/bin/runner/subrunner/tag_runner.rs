/// Functionality for running the test suite contained within a single tag.
/// This contains the functionality for building the project, as well as for
/// iterating over the test cases.
///
use std::{collections::BTreeSet, path::PathBuf, time::Duration};

use id2202_autograder::{
    config::{Tag, TagBuildConfig, Test, TestDefault, TestGroup, Testkind},
    db::models::SubmissionStatusCode,
    error::{Error, ErrorKind, SyscommandError},
    podman,
    reporting::{DetailsBuildFailure, DetailsTagGradingGroup, MIMETypeInfo, ReportTagGrading},
    utils::{self, path_absolute_join, syscommand_timeout, SyscommandSettings},
};
use num_traits::ToPrimitive;

use crate::subrunner::{
    container::ContainerInfo,
    test_grader::{FailureCause, GradingResult},
};

#[derive(Debug, Clone)]
pub enum BuildResult {
    BuildOk,
    BuildTimeout {
        timeout: u32,
        captured_stdout: Option<String>,
        captured_stderr: Option<String>,
    },
    BuildSourceNotFound {
        expected_dir: String,
    },
    BuildOutputLimitExceeded {
        limit: usize,
    },
    BuildProhibitedFiles {
        /// A list of found files in the build dir which are prohibited.
        found_files: Vec<MIMETypeInfo>,
    },
    BuildFailed {
        message: Option<String>,
        code: Option<i32>,
        captured_stdout: Option<String>,
        captured_stderr: Option<String>,
    },
}

/// The runner for a grading tag. This spawns a podman container, builds the
/// project inside the container, and proceeds to run every test case defined
/// for this tag.
#[derive(Debug, Clone)]
pub struct TagRunner {
    /// Information about the container that the tag runner will use to build
    /// the project and grade the tests container within this tag.
    pub container: ContainerInfo,

    /// The name of this tag, e.g. `hello` for the `#hello` tag.
    pub tag_name: String,

    /// Tag groups that this tag was derived from, e.g. `hello` from the
    /// `#hello-all` tag group.
    pub derived_from: BTreeSet<String>,

    /// Build configuration for this specific tag.
    pub build_conf: TagBuildConfig,

    /// Test case defaults, useful for things such as timeout
    pub test_default: TestDefault,

    /// Iterators for each respective test group contained within this tag.
    toplevel_iterator: TestGroupIterator,

    /// Result of the build process
    build_result: Option<BuildResult>,

    /// Where the source files used to grade this solution is located on the
    /// host system. This directory should be considered read-only, and only
    /// ever copied/read from.
    source_dir: String,

    /// Number of failed test cases
    testfail_count: usize,

    /// The result from a test case that means that the entire grading process
    /// should be interrupted.
    bad_test_behavior: Option<FailureCause>,

    /// Number of reports that have been collected for test cases
    pub collected_reports: usize,
}

impl TagRunner {
    /// Creates a new tag runner from a tag specification.
    pub fn new(
        tag: &Tag,
        container: &ContainerInfo,
        test_default: &TestDefault,
        source_dir: &String,
    ) -> Self {
        TagRunner {
            tag_name: tag.name.to_owned(),
            container: container.to_owned(),
            build_conf: tag.build.to_owned(),
            test_default: test_default.to_owned(),
            derived_from: BTreeSet::new(),

            toplevel_iterator: TestGroupIterator::from_groups(
                format!("top-level for tag \"{}\"", tag.name),
                tag.test_groups
                    .iter()
                    .map(|tg| TestGroupIterator::new(tg))
                    .collect(),
            ),

            build_result: None,
            source_dir: source_dir.clone(),
            testfail_count: 0,
            bad_test_behavior: None,
            collected_reports: 0,
        }
    }

    /// Returns true if the solution for this project has been built.
    /// Irregardless of whether it was successfully built or not.
    pub fn has_built(&self) -> bool {
        self.build_result.is_some()
    }

    /// Returns true if a build process has been attempted. This is useful for
    /// checking whether a build was rejected prematurely.
    pub fn attempted_build(&self) -> bool {
        // Note: using an exhaustive match here for the sake of correctness
        match &self.build_result {
            None => false,
            Some(BuildResult::BuildSourceNotFound { .. }) => false,
            Some(BuildResult::BuildProhibitedFiles { .. }) => false,
            Some(BuildResult::BuildOk) => true,
            Some(BuildResult::BuildFailed { .. }) => true,
            Some(BuildResult::BuildTimeout { .. }) => true,
            Some(BuildResult::BuildOutputLimitExceeded { .. }) => true,
        }
    }

    /// Returns a submission status code IF the tag runner has experienced some
    /// bad behavior and the entire grading process should stop.
    pub fn experienced_bad_behavior(&self) -> Option<SubmissionStatusCode> {
        use SubmissionStatusCode as SSC;

        match &self.build_result {
            Some(BuildResult::BuildTimeout { .. }) => Some(SSC::BuildTimedOut),
            Some(BuildResult::BuildOutputLimitExceeded { .. }) => Some(SSC::OutputLimitExceeded),
            None
            | Some(BuildResult::BuildSourceNotFound { .. })
            | Some(BuildResult::BuildProhibitedFiles { .. })
            | Some(BuildResult::BuildOk) => None,
            Some(BuildResult::BuildFailed { .. }) => match self.bad_test_behavior {
                Some(FailureCause::OutputMismatch) => Some(SSC::TestCasesFailed),
                Some(FailureCause::Timeout(_)) => Some(SSC::TestCasesTimedOut),
                Some(FailureCause::OutputLimitExceeded { .. }) => Some(SSC::OutputLimitExceeded),
                None => None,
            },
        }
    }

    /// Compiles a report on the tests within this tag group.
    pub fn results_report(&self) -> ReportTagGrading {
        let build_failure = match &self.build_result {
            None => Some(DetailsBuildFailure { msg: "Never attempted to build the project.".to_string(), ..DetailsBuildFailure::default() }),
            Some(BuildResult::BuildOk) => None, // ok
            Some(BuildResult::BuildSourceNotFound { expected_dir }) => {
                Some(DetailsBuildFailure {
                    msg: "Could not build the project.".to_string(),
                    srcdir: Some(expected_dir.clone()),
                    missing_source_directory: true,
                    ..DetailsBuildFailure::default()
                })
            }
            Some(BuildResult::BuildProhibitedFiles { found_files }) => {
                Some(DetailsBuildFailure {
                    msg: "Build failed due to unexpected non-text files in your solution."
                        .to_string(),
                    srcdir: Some(self.build_conf.srcdir.clone()),
                    prohibited_mimetype_files: found_files.clone(),
                    suffix_message: Some("Please remove these files from your solution directory and make sure that your .gitignore is properly configured.".to_string()),
                    ..DetailsBuildFailure::default()
                })
            }
            Some(BuildResult::BuildFailed {
                message,
                code,
                captured_stdout,
                captured_stderr,
            }) => {
                let mut desc = "Build process failed.".to_string();
                if let Some(msg) = message {
                    desc.push_str(&format!(" {}", msg));
                }
                Some(DetailsBuildFailure {
                    msg: desc,
                    cmd: Some(self.build_conf.cmd.join(" ")),
                    srcdir: Some(self.build_conf.srcdir.clone()),
                    exit_code: code.clone(),
                    captured_stdout: captured_stdout.clone(),
                    captured_stderr: captured_stderr.clone(),
                    ..DetailsBuildFailure::default()
                })
            }
            Some(BuildResult::BuildTimeout {
                timeout,
                captured_stdout,
                captured_stderr,
            }) => Some(DetailsBuildFailure {
                msg: format!("Build process timed out after {} seconds.", timeout),
                cmd: Some(self.build_conf.cmd.join(" ")),
                srcdir: Some(self.build_conf.srcdir.clone()),
                captured_stdout: captured_stdout.clone(),
                captured_stderr: captured_stderr.clone(),
                ..DetailsBuildFailure::default()
            }),
            Some(BuildResult::BuildOutputLimitExceeded { limit }) => {
                Some(DetailsBuildFailure {
                    msg: format!(
                        "Build failed due to exceeding the output limit of {} bytes on standard output or standard error.",
                        limit
                    ),
                    cmd: Some(self.build_conf.cmd.join(" ")),
                    srcdir: Some(self.build_conf.srcdir.clone()),
                    ..DetailsBuildFailure::default()
                })
            }
        };
        let mut ok = build_failure.is_none();
        let mut group_results = vec![];
        if ok {
            for sg in &self.toplevel_iterator.subgroup_iterators {
                let (res, all_ok) = sg.group_details();
                ok &= all_ok;
                group_results.push(res);
            }
        }
        ReportTagGrading {
            tag_name: self.tag_name.clone(),
            derived_from: self.derived_from.iter().cloned().collect(),
            build_failure: build_failure,
            ok: ok,
            groups: group_results,
        }
    }

    /// Removes the podman container if it is still running and makes sure that
    /// the build directory is removed.
    pub fn cleanup(&mut self) -> Result<(), Error> {
        if podman::ps_names()?.contains(&self.container.podman_container_name) {
            log::debug!(
                "Removing the container used for grading \"{}\"",
                self.tag_name
            );
            podman::force_rm(&self.container.podman_container_name)?;
        }

        if std::fs::exists(&self.container.external_solution)? {
            log::debug!(
                "Removing the build directory used for grading \"{}\"",
                self.tag_name
            );
            std::fs::remove_dir_all(&self.container.external_solution)?;
        }

        Ok(())
    }

    /// Build the solution for this grading tag. This must be performed before
    /// the solution can be graded. Returns Ok(`true`) if the project was built
    /// successfully, Ok(`false`) if there was an issue building the project.
    /// `Err` is only returned if an internal error occurred on the autograder
    /// side, and the grading process must be interrupted.
    ///
    /// This function also spawns the podman container used to grade all the
    /// tests inside this tag. This container remains active for the duration
    /// of the tag grading procedure.
    pub fn build(&mut self) -> Result<bool, Error> {
        if self.build_result.is_some() {
            return Error::err_runtime(format!(
                "Attempted to build project twice for tag \"{}\"",
                &self.tag_name
            ));
        }

        log::info!(
            "Building project for tag \"{}\" (src: {})",
            self.tag_name,
            self.build_conf.srcdir
        );

        let running_containers = podman::ps_names()?;
        if running_containers.contains(&self.container.podman_container_name) {
            log::warn!("Removing dangling image from previous run");
            podman::force_rm(&self.container.podman_container_name)?;
        }

        if std::fs::exists(&self.container.external_solution)? {
            // Remove the old build dir
            std::fs::remove_dir_all(&self.container.external_solution)?;
        }
        if !std::fs::exists(&self.container.external_tests)? {
            // Ensure that the test directory exists outside the container
            std::fs::create_dir_all(&self.container.external_tests)?;
        }

        // Copy the solution directory to the <workspace>/build
        let solution_dir: String = path_absolute_join(&self.source_dir, &self.build_conf.srcdir)?;

        if !std::fs::exists(&solution_dir)? {
            self.build_result.replace(BuildResult::BuildSourceNotFound {
                expected_dir: self.build_conf.srcdir.to_owned(),
            });
            return Ok(false);
        }

        dircpy::copy_dir(&solution_dir, &self.container.external_solution)?;

        // Check for forbidden binary files inside the solution directory
        // (Path, mime output)
        let mut forbidden_files: Vec<MIMETypeInfo> = vec![];
        fn recur_scandir(
            forbidden_files: &mut Vec<MIMETypeInfo>,
            allowed_files: &Vec<String>,
            allowed_mimetypes: &Vec<String>,
            dir: PathBuf,
            dir_prefix: String,
        ) -> Result<(), Error> {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let entry_name = entry
                    .file_name()
                    .into_string()
                    .map_err(|oss| Error::parse_type("utf-8 filename", format!("{oss:?}")))?;
                let path = format!("{}{}", dir_prefix, entry_name);
                if allowed_files.contains(&path) {
                    log::info!("Found allowed binary file: {path}");
                    continue;
                }
                if entry.file_type()?.is_dir() {
                    recur_scandir(
                        forbidden_files,
                        allowed_files,
                        allowed_mimetypes,
                        entry.path(),
                        format!("{}{}/", dir_prefix, entry_name),
                    )?;
                } else {
                    // Check mime-type of this file
                    let mimetype = utils::mimetype(&entry.path())?;
                    if allowed_mimetypes
                        .iter()
                        .any(|prefix| mimetype.starts_with(prefix))
                    {
                        log::info!(
                        "Found allowed binary file: {path} (due to allowed MIME type {mimetype})"
                    );
                        continue;
                    }
                    if !mimetype.starts_with("text/") {
                        log::error!("Found forbidden file: {:?}", entry.path());
                        forbidden_files.push(MIMETypeInfo {
                            path: path,
                            mime_identified: mimetype,
                            ..Default::default()
                        });
                    }
                }
            }
            Ok(())
        }
        if self.build_conf.prohibit_binary_files {
            log::debug!("Checking for prohibited files.");
            recur_scandir(
                &mut forbidden_files,
                &self.build_conf.allowed_binary_files,
                &self.build_conf.allowed_binary_mimetypes,
                PathBuf::from(&self.container.external_solution),
                "".to_string(),
            )
            .inspect_err(|e| log::error!("Error when scanning for prohibited files: {e}"))?;
        }
        if forbidden_files.len() > 0 {
            self.build_result
                .replace(BuildResult::BuildProhibitedFiles {
                    found_files: forbidden_files,
                });
            return Ok(false);
        }

        log::debug!("Starting podman container");
        podman::start_container(&podman::ContainerOptions {
            image: self.container.podman_image.to_owned(),
            container_name: self.container.podman_container_name.to_owned(),
            network_name: self.container.podman_network_name.to_owned(),
            mounts: vec![
                (
                    self.container.external_solution.to_owned(),
                    self.container.mount_solution.to_owned(),
                    "ro,z".to_string(),
                ),
                (
                    self.container.external_tests.to_owned(),
                    self.container.mount_tests.to_owned(),
                    "ro,z".to_string(),
                ),
            ],
        })?;

        // Wait for the container to start
        let mut start_attempts = 0;
        let mut container_started = false;
        while !container_started {
            start_attempts += 1;
            if start_attempts > 10 {
                return Error::err_runtime("container would not start after 10 attempts");
            }
            for ps_output in podman::ps()?.iter() {
                if ps_output
                    .names
                    .contains(&self.container.podman_container_name)
                    && ps_output.state == "running"
                {
                    container_started = true;
                }
            }
            if !container_started {
                std::thread::sleep(Duration::from_millis(500));
            }
        }

        // Double-check that the target repo doesn't exist
        podman::exec(
            &self.container.podman_container_name,
            &["test", "!", "-d", &self.container.internal_build_dir],
        )?;

        // Now copy the solution to the root repository
        podman::exec(
            &self.container.podman_container_name,
            &[
                "cp",
                "-r",
                &self.container.mount_solution,
                &self.container.internal_build_dir,
            ],
        )?;

        let mut build_cmd: Vec<&str> = vec![
            "podman",
            "exec",
            "-w",
            &self.container.internal_build_dir,
            &self.container.podman_container_name,
        ];
        build_cmd.extend(self.build_conf.cmd.iter().map(String::as_str));

        log::info!("Starting build {build_cmd:?}");

        match syscommand_timeout(
            build_cmd.as_slice(),
            SyscommandSettings {
                max_stdout_length: Some(self.test_default.max_output),
                max_stderr_length: Some(self.test_default.max_output),
                timeout: Duration::from_secs(self.build_conf.timeout.into()),
                ..Default::default()
            },
        ) {
            Ok(output) => {
                if output.code == 0 {
                    self.build_result.replace(BuildResult::BuildOk);
                } else {
                    self.build_result.replace(BuildResult::BuildFailed {
                        message: None,
                        code: Some(output.code),
                        captured_stdout: Some(output.stdout),
                        captured_stderr: Some(output.stderr),
                    });
                    return Ok(false);
                }
            }
            Err(Error {
                kind:
                    ErrorKind::Syscommand(SyscommandError {
                        timeout: Some(_),
                        stdout,
                        stderr,
                        ..
                    }),
                ..
            }) => {
                self.build_result.replace(BuildResult::BuildTimeout {
                    timeout: self.build_conf.timeout,
                    captured_stdout: stdout,
                    captured_stderr: stderr,
                });
                return Ok(false);
            }
            Err(Error {
                kind:
                    ErrorKind::Syscommand(SyscommandError {
                        output_limit_exceeded: Some(limit),
                        ..
                    }),
                ..
            }) => {
                self.build_result
                    .replace(BuildResult::BuildOutputLimitExceeded { limit: limit });
                return Ok(false);
            }
            Err(e) => {
                log::error!("Error running build command: {e}");
                return Err(e);
            }
        }
        log::info!("Build finished. Disconnecting network from container.");

        // Now disconnect the container from the network
        syscommand_timeout(
            &[
                "podman",
                "network",
                "disconnect",
                &self.container.podman_network_name,
                &self.container.podman_container_name,
            ],
            SyscommandSettings {
                expected_code: Some(0),
                ..Default::default()
            },
        )?;

        log::info!("Proceeding to run test cases.");

        // If the build was successful, we set up the iterator to point at the first test case
        self.toplevel_iterator.next();

        Ok(true)
    }

    /// Runs the next test case. Returns `true` if there are more test cases to
    /// run. Returns `false` if we have run the final test case.
    ///
    /// If `include_report` is true, then a failure report will be collected
    /// if this test case fails.
    pub fn run_test(&mut self, include_report: bool) -> Result<bool, Error> {
        // First validate the solution is built
        match &self.build_result {
            Some(BuildResult::BuildOk) => {} // OK
            Some(_) => {
                return Error::err_runtime(format!(
                    "Attempted to run a test case for tag \"{}\" following a failed build process",
                    self.tag_name
                ));
            }
            None => {
                return Error::err_runtime(format!(
                    "Attempted to run a test case for tag \"{}\" without first building the project",
                    self.tag_name
                ));
            }
        }

        let test = match self.toplevel_iterator.peek() {
            Some(t) => t,
            None => {
                return Error::err_runtime(format!(
                    "Attempted to run a test case for tag \"{}\", but no test could be found",
                    self.tag_name
                ));
            }
        };

        let result = match &test.kind {
            Testkind::Run(conf) => {
                use crate::subrunner::test_grader::Run;
                Run::grade_from_testkind(conf, &self.test_default, &self.container, include_report)?
            }
            Testkind::GenASMAndRun(conf) => {
                use crate::subrunner::test_grader::GenASMAndRun;
                GenASMAndRun::grade_from_testkind(
                    conf,
                    &self.test_default,
                    &self.container,
                    include_report,
                )?
            }
            Testkind::CheckFileExists(conf) => {
                use crate::subrunner::test_grader::CheckFileExists;
                CheckFileExists::grade_from_testkind(
                    conf,
                    &self.test_default,
                    &self.container,
                    include_report,
                )?
            }
        };

        match &result {
            GradingResult::Success { captured_stdout: _ } => {} // ok
            GradingResult::Failure { cause, report } => {
                self.testfail_count += 1;
                if let Some(_) = report {
                    self.collected_reports += 1;
                }
                match cause {
                    FailureCause::OutputMismatch => {}
                    FailureCause::Timeout(d) => {
                        log::debug!("Test timed out after {} seconds", d.as_secs());
                        self.bad_test_behavior.replace(cause.clone());
                    }
                    FailureCause::OutputLimitExceeded { limit } => {
                        log::debug!("Test output exceeded {} bytes", limit);
                        self.bad_test_behavior.replace(cause.clone());
                    }
                }
            }
        }
        self.toplevel_iterator.add_result(result)?;

        // Progress to the next test case before returning
        Ok(self.bad_test_behavior.is_none() && self.toplevel_iterator.next())
    }
}

/// Iterator for running the tests in a test group and all tests in the
/// contained subgroups.
///
/// It will first run any tests contained inside the subgroups. After that has
/// finished, it will run any tests contained directly inside this test group
/// as well.
///
/// Before a test can be run, `next()` has to be called first.
#[derive(Debug, Clone)]
struct TestGroupIterator {
    /// Metadata from the testgroup
    pub title: String,

    subgroup_iterators: Vec<TestGroupIterator>,
    next_subgroup: usize,

    /// This is an isize, so -1 means that we have not yet checked the first
    /// test.
    next_test_idx: isize,

    tests: Vec<Test>,
    results: Vec<GradingResult>,
}

impl TestGroupIterator {
    /// Creates a new iterator from a test group
    fn new(tg: &TestGroup) -> Self {
        TestGroupIterator {
            title: tg.title.to_owned(),
            subgroup_iterators: tg.subgroups.iter().map(|sg| Self::new(sg)).collect(),
            next_subgroup: 0,
            next_test_idx: -1,
            tests: tg.tests.to_owned(),
            results: vec![],
        }
    }

    fn from_groups(title: String, groups: Vec<TestGroupIterator>) -> Self {
        TestGroupIterator {
            title: title,
            subgroup_iterators: groups,
            next_subgroup: 0,
            next_test_idx: -1,
            tests: vec![],
            results: vec![],
        }
    }

    /// Returns the next test to run. Returns None if there is not a next test
    /// to run or if `TestGroupIterator::next()` has not yet been invoked.
    fn peek(&self) -> Option<&Test> {
        if let Some(Some(t_opt)) = self
            .subgroup_iterators
            .get(self.next_subgroup)
            .map(|sg| sg.peek())
        {
            return Some(t_opt);
        }

        // If we have not yet called next()
        if self.next_test_idx < 0 {
            return None;
        }

        self.next_test_idx
            .to_usize()
            .and_then(|i| self.tests.get(i))
    }

    /// Progresses to the next test case. Returns `true` if there is a new test
    /// to run. Returns `false` if we are at the end and there are no more
    /// tests to run for this tag group.
    fn next(&mut self) -> bool {
        while self.next_subgroup < self.subgroup_iterators.len() {
            let subgroup = self.subgroup_iterators.get_mut(self.next_subgroup).unwrap();
            if subgroup.next() {
                return true;
            }
            self.next_subgroup += 1;
        }

        // After the last subgroup has finished, next_test_idx should be -1, so
        // then we increment it to 0 to signal the start of the first test.

        if self.next_test_idx < self.tests.len().to_isize().unwrap_or(isize::MAX) {
            self.next_test_idx += 1;
            return self.next_test_idx < self.tests.len().to_isize().unwrap_or(isize::MAX);
        }

        return false;
    }

    /// Adds the test result from a run
    fn add_result(&mut self, res: GradingResult) -> Result<(), Error> {
        if let Some(sg) = self.subgroup_iterators.get_mut(self.next_subgroup) {
            return sg.add_result(res);
        }
        if self.results.len().to_isize().unwrap_or(isize::MAX) != self.next_test_idx {
            return Error::err_runtime(format!(
                    "Internal error: Adding result to the wrong test case. At test {}, added result to {}.",
                    self.next_test_idx,
                    self.results.len()
                ));
        }
        self.results.push(res);
        Ok(())
    }

    /// Compiles the details necessary for this tag grading group, as well as
    /// indicating whether everything was successful or not.
    fn group_details(&self) -> (DetailsTagGradingGroup, bool) {
        let mut all_ok = true;
        let mut sg_details = vec![];
        for sg in &self.subgroup_iterators {
            let (sg_d, sg_ok) = sg.group_details();
            all_ok &= sg_ok;
            sg_details.push(sg_d);
        }
        let details = DetailsTagGradingGroup {
            group_title: self.title.clone(),
            subgroups: sg_details,
            local_tests: self.tests.len(),
            tests_run: self.results.len(),
            tests_passed: self
                .results
                .iter()
                .filter(|r| match r {
                    GradingResult::Success { .. } => true,
                    _ => false,
                })
                .count(),
            test_details: self
                .results
                .iter()
                .filter_map(|r| match r {
                    GradingResult::Failure { cause: _, report } => report.clone().map(|b| *b),
                    _ => None,
                })
                .collect(),
        };
        all_ok &= details.local_tests == details.tests_passed;
        (details, all_ok)
    }
}
