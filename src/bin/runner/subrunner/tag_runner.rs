/// Functionality for running the test suite contained within a single tag.
/// This contains the functionality for building the project, as well as for
/// iterating over the test cases.
///
use std::{
    rc::Rc,
    time::{Duration, SystemTime},
};

use id2202_autograder::{
    config::{tests::kind::Kind as Testkind, BuildConfig, Settings, Tag, Test, TestGroup},
    db::models::{JobStatus, SubmissionJobPlain},
    error::{Error, ErrorKind, SyscommandError},
    podman,
    reporting::{DetailsBuildFailure, DetailsTagGradingGroup, MIMETypeInfo, ReportTagGrading},
    utils::{self, path_absolute_join, syscommand_timeout, SyscommandSettings},
};
use num_traits::ToPrimitive;
use walkdir::WalkDir;

use crate::{
    shadow::ShadowRepo,
    subrunner::{
        container::ContainerInfo,
        test_grader::{FailureCause, GradingResult},
        verifier,
    },
};

#[derive(Debug, Clone)]
pub enum BuildResult {
    Ok,
    Timeout {
        timeout: u32,
        captured_stdout: Option<String>,
        captured_stderr: Option<String>,
    },
    SourceNotFound {
        expected_dir: String,
    },
    OutputLimitExceeded {
        limit: usize,
    },
    ProhibitedFiles {
        /// A list of found files in the build dir which are prohibited.
        found_files: Vec<MIMETypeInfo>,
    },
    Failed {
        message: Option<String>,
        code: Option<i32>,
        captured_stdout: Option<String>,
        captured_stderr: Option<String>,
    },
}

/// The runner for a grading tag. This spawns a podman container, builds the
/// project inside the container, and proceeds to run every test case defined
/// for this tag.
#[derive(Debug)]
pub struct TagRunner<'a> {
    /// Program settings, passed on to the test kinds that need them.
    pub settings: &'a Settings,

    /// Information about the container that the tag runner will use to build
    /// the project and grade the tests container within this tag.
    pub container: ContainerInfo,

    /// The job this tag runner is grading, which is also where its tag name
    /// and the names it was requested as come from.
    pub job: SubmissionJobPlain,

    /// Build configuration for this specific tag.
    pub build_conf: BuildConfig,

    /// The verifiers, shared with every other tag runner of this submission.
    verifier: Rc<verifier::Verifier>,

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

    /// How far this tag has got. Grading of the tag is over once this is
    /// terminal, whether it succeeded or not.
    pub status: JobStatus,

    /// The report for this tag, once `generate_report` has compiled it.
    generated_report: Option<ReportTagGrading>,

    /// How long this tag may run for once it starts.
    timeout_total: Duration,

    /// When grading of this tag has to be over. Provisional while the status
    /// is `NotStarted`, and stamped for real by `start`.
    deadline: SystemTime,
}

impl<'a> TagRunner<'a> {
    /// Creates a new tag runner from a tag specification.
    pub fn new(
        settings: &'a Settings,
        tag: &Tag,
        job: SubmissionJobPlain,
        container: ContainerInfo,
        source_dir: &str,
        verifier: Rc<verifier::Verifier>,
        timeout_total: Duration,
    ) -> Self {
        TagRunner {
            settings,
            job,
            container,
            build_conf: tag.build.to_owned(),
            verifier,

            toplevel_iterator: TestGroupIterator::from_groups(
                format!("top-level for tag \"{}\"", tag.name),
                tag.test_groups.iter().map(TestGroupIterator::new).collect(),
            ),

            build_result: None,
            source_dir: source_dir.to_owned(),
            testfail_count: 0,
            bad_test_behavior: None,
            collected_reports: 0,
            status: JobStatus::NotStarted,
            generated_report: None,
            timeout_total,
            deadline: SystemTime::UNIX_EPOCH,
        }
    }

    /// Whether the tag has run past the deadline stamped when it started.
    ///
    /// # Note
    /// A tag that has not started is not considered to have exceeded its
    /// deadline.
    fn exceeded_deadline(&self) -> bool {
        self.status != JobStatus::NotStarted && SystemTime::now() >= self.deadline
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
            Some(BuildResult::SourceNotFound { .. }) => false,
            Some(BuildResult::ProhibitedFiles { .. }) => false,
            Some(BuildResult::Ok) => true,
            Some(BuildResult::Failed { .. }) => true,
            Some(BuildResult::Timeout { .. }) => true,
            Some(BuildResult::OutputLimitExceeded { .. }) => true,
        }
    }

    /// Returns a job status IF the tag runner has experienced some bad
    /// behavior and that the grading of this tag should stop.
    pub fn experienced_bad_behavior(&self) -> Option<JobStatus> {
        match &self.build_result {
            Some(BuildResult::Timeout { .. }) => Some(JobStatus::BuildTimedOut),
            Some(BuildResult::OutputLimitExceeded { .. }) => {
                Some(JobStatus::BuildOutputLimitExceeded)
            }
            None
            | Some(BuildResult::SourceNotFound { .. })
            | Some(BuildResult::ProhibitedFiles { .. })
            | Some(BuildResult::Failed { .. }) => None,
            Some(BuildResult::Ok) => match self.bad_test_behavior {
                Some(FailureCause::OutputMismatch) => Some(JobStatus::TestCasesFailed),
                Some(FailureCause::Timeout(_)) => Some(JobStatus::TestCasesTimedOut),
                Some(FailureCause::OutputLimitExceeded { .. }) => {
                    Some(JobStatus::TestOutputLimitExceeded)
                }
                None => None,
            },
        }
    }

    /// Compiles the report on the tests within this tag group, and keeps it
    /// for `get_report`. This ends grading of the tag, settling its status
    /// unless one was already reached. Errors if the report has already been
    /// compiled.
    pub fn generate_report(&mut self) -> Result<&ReportTagGrading, Error> {
        if self.generated_report.is_some() {
            return Error::err_runtime(format!(
                "Attempted to generate the report for tag \"{}\" twice",
                self.job.tag
            ));
        }

        let build_failure = match &self.build_result {
            None => Some(DetailsBuildFailure { msg: "Never attempted to build the project.".to_string(), ..DetailsBuildFailure::default() }),
            Some(BuildResult::Ok) => None, // ok
            Some(BuildResult::SourceNotFound { expected_dir }) => {
                Some(DetailsBuildFailure {
                    msg: "Could not build the project.".to_string(),
                    srcdir: Some(expected_dir.clone()),
                    missing_source_directory: true,
                    ..DetailsBuildFailure::default()
                })
            }
            Some(BuildResult::ProhibitedFiles { found_files }) => {
                Some(DetailsBuildFailure {
                    msg: "Build failed due to unexpected non-text files in your solution."
                        .to_string(),
                    srcdir: Some(self.build_conf.srcdir.clone()),
                    prohibited_mimetype_files: found_files.clone(),
                    suffix_message: Some("Please remove these files from your solution directory and make sure that your .gitignore is properly configured.".to_string()),
                    ..DetailsBuildFailure::default()
                })
            }
            Some(BuildResult::Failed {
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
                    exit_code: *code,
                    captured_stdout: captured_stdout.clone(),
                    captured_stderr: captured_stderr.clone(),
                    ..DetailsBuildFailure::default()
                })
            }
            Some(BuildResult::Timeout {
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
            Some(BuildResult::OutputLimitExceeded { limit }) => {
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

        // A status reached earlier, such as a timeout, says more about how the
        // tag ended than the results do, so it is left alone.
        if !self.status.is_finished() {
            self.status = self.experienced_bad_behavior().unwrap_or(if ok {
                JobStatus::Success
            } else if build_failure.is_some() {
                JobStatus::BuildError
            } else {
                JobStatus::TestCasesFailed
            });
        }

        let report = ReportTagGrading {
            tag_name: self.job.tag.clone(),
            derived_from: self.job.requested_as.clone(),
            premature_exit_reason: match self.status {
                JobStatus::JobTimedOut => Some("Grading of this tag timed out.".to_string()),
                JobStatus::AutograderFailure => {
                    Some("Grading process was interrupted. Contact course staff.".to_string())
                }
                _ => None,
            },
            build_failure,
            ok,
            groups: group_results,
        };

        Ok(self.generated_report.insert(report))
    }

    /// The report for this tag, if `generate_report` has compiled it.
    pub fn get_report(&self) -> Option<&ReportTagGrading> {
        self.generated_report.as_ref()
    }

    /// Removes the podman container if it is still running and makes sure that
    /// the build directory is removed.
    pub fn cleanup(&mut self) -> Result<(), Error> {
        self.container.podman.stop();

        if std::fs::exists(&self.container.solution_dir.host_path)? {
            log::debug!("Removing the build directory used for grading \"{}\"", self.job.tag);
            std::fs::remove_dir_all(&self.container.solution_dir.host_path)?;
        }

        Ok(())
    }

    /// Build the solution for this grading tag. This must be performed before
    /// the solution can be graded. Returns Ok(`true`) if the project was built
    /// successfully, Ok(`false`) if there was an issue building the project.
    /// `Err` is only returned if an internal error occurred on the autograder
    /// side, and the grading process must be interrupted.
    ///
    /// This function is considered the start of the grading process, and will
    /// also set the deadline.
    ///
    /// This function also spawns the podman container used to grade all the
    /// tests inside this tag. This container remains active for the duration
    /// of the tag grading procedure.
    pub fn build(&mut self) -> Result<bool, Error> {
        if self.build_result.is_some() || self.status != JobStatus::NotStarted {
            return Error::err_runtime(format!(
                "Attempted to build project twice for tag \"{}\"",
                self.job.tag
            ));
        }

        // Building is where the tag starts, so its deadline runs from here.
        self.deadline = SystemTime::now().checked_add(self.timeout_total).ok_or_else(|| {
            Error::runtime(format!("could not set a deadline for tag \"{}\"", self.job.tag))
        })?;
        self.status = JobStatus::Running;

        log::info!(
            "Building project for tag \"{}\" (src: {})",
            self.job.tag,
            self.build_conf.srcdir
        );

        let running_containers = podman::ps_names()?;
        if running_containers.contains(&self.container.podman.name) {
            log::warn!("Removing dangling image from previous run");
            podman::force_rm(&self.container.podman.name)?;
        }

        if std::fs::exists(&self.container.solution_dir.host_path)? {
            // Remove the old build dir
            std::fs::remove_dir_all(&self.container.solution_dir.host_path)?;
        }
        if !std::fs::exists(&self.container.tests_dir.host_path)? {
            // Ensure that the test directory exists outside the container
            std::fs::create_dir_all(&self.container.tests_dir.host_path)?;
        }

        // Copy the solution directory to the <workspace>/build
        let solution_dir: String = path_absolute_join(&self.source_dir, &self.build_conf.srcdir)?;

        if !std::fs::exists(&solution_dir)? {
            self.build_result.replace(BuildResult::SourceNotFound {
                expected_dir: self.build_conf.srcdir.to_owned(),
            });
            return Ok(false);
        }

        dircpy::copy_dir(&solution_dir, &self.container.solution_dir.host_path)?;

        // Check for forbidden binary files inside the solution directory
        let mut forbidden_files: Vec<MIMETypeInfo> = vec![];
        if self.build_conf.prohibit_binary_files {
            log::debug!("Checking for prohibited files.");
            let scan_root = &self.container.solution_dir.host_path;
            for entry in WalkDir::new(scan_root) {
                let entry = entry.inspect_err(|e| {
                    log::error!("Error when scanning for prohibited files: {e}")
                })?;
                if !entry.file_type().is_file() {
                    continue;
                }

                let path = entry
                    .path()
                    .strip_prefix(scan_root)
                    .ok()
                    .and_then(|p| p.to_str())
                    .ok_or_else(|| {
                        Error::parse_type("invalid utf-8 path", format!("{:?}", entry.path()))
                    })?
                    .to_string();

                if self.build_conf.allowed_binary_files.contains(&path) {
                    log::info!("Skipping allowed binary file path: {path}");
                    continue;
                }

                let mimetype = utils::mimetype(entry.path())?;
                if self
                    .build_conf
                    .allowed_binary_mimetypes
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
                        path,
                        mime_identified: mimetype,
                        ..Default::default()
                    });
                }
            }
        }
        if !forbidden_files.is_empty() {
            self.build_result
                .replace(BuildResult::ProhibitedFiles { found_files: forbidden_files });
            return Ok(false);
        }

        log::debug!("Starting podman container");
        self.container.podman.mounts =
            vec![self.container.solution_dir.clone(), self.container.tests_dir.clone()];
        self.container.podman.start()?;

        // Wait for the container to start
        let mut start_attempts = 0;
        let mut container_started = false;
        while !container_started {
            start_attempts += 1;
            if start_attempts > 10 {
                return Error::err_runtime("container would not start after 10 attempts");
            }
            for ps_output in podman::ps()?.iter() {
                if ps_output.names.contains(&self.container.podman.name)
                    && ps_output.state == "running"
                {
                    container_started = true;
                }
            }
            if !container_started {
                std::thread::sleep(Duration::from_millis(500));
            }
        }

        let checked = SyscommandSettings {
            expected_code: Some(0),
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        };

        // Double-check that the target repo doesn't exist
        self.container.podman.exec(
            None,
            ["test", "!", "-d", &self.container.internal_build_dir],
            checked.clone(),
        )?;

        // Now copy the solution to the root repository
        self.container.podman.exec(
            None,
            [
                "cp",
                "-r",
                &self.container.solution_dir.container_path,
                &self.container.internal_build_dir,
            ],
            checked,
        )?;

        let build_cmd: Vec<&str> = self.build_conf.cmd.iter().map(String::as_str).collect();

        log::info!("Starting build {build_cmd:?}");

        match self.container.podman.exec(
            Some(&self.container.internal_build_dir),
            build_cmd.as_slice(),
            SyscommandSettings {
                max_stdout_length: Some(self.build_conf.max_output),
                max_stderr_length: Some(self.build_conf.max_output),
                timeout: Duration::from_secs(self.build_conf.timeout.into()),
                ..Default::default()
            },
        ) {
            Ok(output) => {
                if output.code == 0 {
                    self.build_result.replace(BuildResult::Ok);
                } else {
                    self.build_result.replace(BuildResult::Failed {
                        message: None,
                        code: Some(output.code),
                        captured_stdout: Some(output.stdout),
                        captured_stderr: Some(output.stderr),
                    });
                    return Ok(false);
                }
            }
            Err(mut boxed_e) => match boxed_e.kind.as_mut() {
                ErrorKind::Syscommand(SyscommandError {
                    timeout: Some(_), stdout, stderr, ..
                }) => {
                    self.build_result.replace(BuildResult::Timeout {
                        timeout: self.build_conf.timeout,
                        captured_stdout: stdout.take(),
                        captured_stderr: stderr.take(),
                    });
                    return Ok(false);
                }
                ErrorKind::Syscommand(SyscommandError {
                    output_limit_exceeded: Some(limit),
                    ..
                }) => {
                    self.build_result.replace(BuildResult::OutputLimitExceeded { limit: *limit });
                    return Ok(false);
                }
                _ => {
                    log::error!("Error running build command: {boxed_e}");
                    return Err(boxed_e);
                }
            },
        }
        log::info!("Build finished. Disconnecting network from container.");

        // Now disconnect the container from the network
        syscommand_timeout(
            [
                "podman",
                "network",
                "disconnect",
                self.container.podman.network.as_deref().unwrap_or("none"),
                &self.container.podman.name,
            ],
            SyscommandSettings { expected_code: Some(0), ..Default::default() },
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
        use crate::subrunner::test_grader::grade;

        // The tag gets a total budget, on top of the per-build and per-test
        // timeouts. It is spent between test cases rather than during one.
        if self.exceeded_deadline() {
            log::info!("Grading of tag \"{}\" timed out", self.job.tag);
            self.status = JobStatus::JobTimedOut;
            return Ok(false);
        }

        // First validate the solution is built
        match &self.build_result {
            Some(BuildResult::Ok) => {} // OK
            Some(_) => {
                return Error::err_runtime(format!(
                    "Attempted to run a test case for tag \"{}\" following a failed build process",
                    self.job.tag
                ));
            }
            None => {
                return Error::err_runtime(format!(
                    "Attempted to run a test case for tag \"{}\" without first building the project",
                    self.job.tag
                ));
            }
        }

        let test = match self.toplevel_iterator.peek() {
            Some(t) => t,
            None => {
                return Error::err_runtime(format!(
                    "Attempted to run a test case for tag \"{}\", but no test could be found",
                    self.job.tag
                ));
            }
        };

        let result = match &test.kind {
            Testkind::Run(conf) => grade::run(conf, test, &self.container, include_report)?,
            Testkind::GenASMAndRun(conf) => {
                grade::gen_asm_and_run(self.settings, conf, test, &self.container, include_report)?
            }
            Testkind::CheckFileExists(conf) => {
                grade::check_file_exists(conf, &self.container, include_report)?
            }
            Testkind::RunVerifier(conf) => grade::run_verifier(
                conf,
                test,
                &self.container,
                &self.verifier,
                self.verifier.container_path(&conf.verifier_path)?,
                include_report,
            )?,
        };

        match &result {
            GradingResult::Success => {} // ok
            GradingResult::Failure { cause, report } => {
                self.testfail_count += 1;
                if report.is_some() {
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

    /// Records information about this tag to the shadow repository, storing a
    /// report about this tag's result and a snapshot of the code that was
    /// graded.
    pub fn record_to_shadow(&self, shadow: &ShadowRepo) -> Result<(), Error> {
        let report = self.get_report().ok_or_else(|| {
            Error::runtime(format!(
                "Attempted to record tag \"{}\" to the shadow before report was generated",
                self.job.tag
            ))
        })?;
        shadow.write_result(report)?;

        // The code is intentionally left out when no build was attempted, as
        // there may be unapproved binary files in the source directory.
        if self.attempted_build() {
            shadow.snapshot(&self.source_dir, &self.build_conf.srcdir)?;
        }

        shadow.commit(&format!(
            "Submission {}: results for tag {}",
            shadow.submission_id, self.job.tag
        ))
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
            subgroup_iterators: tg.subgroups.iter().map(Self::new).collect(),
            next_subgroup: 0,
            next_test_idx: -1,
            tests: tg.tests.to_owned(),
            results: vec![],
        }
    }

    fn from_groups(title: String, groups: Vec<TestGroupIterator>) -> Self {
        TestGroupIterator {
            title,
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
        if let Some(Some(t_opt)) =
            self.subgroup_iterators.get(self.next_subgroup).map(|sg| sg.peek())
        {
            return Some(t_opt);
        }

        // If we have not yet called next()
        if self.next_test_idx < 0 {
            return None;
        }

        self.next_test_idx.to_usize().and_then(|i| self.tests.get(i))
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

        false
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
                .filter(|r| matches!(r, GradingResult::Success))
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
