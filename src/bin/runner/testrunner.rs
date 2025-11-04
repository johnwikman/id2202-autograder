// Podman functionality
// (Probably good to rename this as testrunner.rs)
// (... with dedicated build and run_test functions.)

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use id2202_autograder::{
    db::models::{Submission, SubmissionStatusCode},
    error::Error,
    podman,
    settings::{RunnerMarkdownSettings, Settings},
    test_config::{Tag, TagBuildConfig, Test, TestGroup, Testkind, Tests},
    utils::{
        self, md_preformatted, md_preformatted_with_truncation, path_absolute_join,
        syscommand_timeout, systemtime_to_utc_string, SyscommandSettings,
    },
};
use itertools::Itertools;

static ERRMSG_INTERNAL_ERROR: &str = "Internal error when starting job. Contact course staff.";

#[derive(Debug, Clone)]
pub struct TestRunnerHandle {
    /// The image which is used to run the container.
    pub podman_image: String,

    /// The name of the container.
    pub podman_container_name: String,

    /// The network to attach to the container.
    pub podman_network_name: String,

    /// Path inside the container which the repo (solution dir) is mounted from.
    pub mount_repo: String,

    /// Path inside the container which is used to place tests in.
    pub mount_tests: String,

    /// Maxmimum length of output from stderr or stdout. Test case will fail if
    /// the output is longer than this.
    pub max_output: usize,

    /// Truncate captured output that exceeds this length when showing it in
    /// the formatted markdown.
    pub truncate_len: usize,

    /// The directory in which this runner will place artifacts. E.g. cloned
    /// git repositories here, input files for test cases, etc. This directory
    /// should be removed by calling the `cleanup` function.
    pub workspace: String,

    /// ID of the current submission.
    pub submission_id: i64,

    /// The path to the cloned source repository. The runner will not modify
    /// this directory after cloning, and will instead copy the part of the
    /// repository that is needed to grade a specfific tag.
    pub repo_dir: String,

    /// The commit that was graded.
    repo_commit: String,

    // Internal state variables below
    /// The next tag to run
    next_tag_index: usize,

    /// Test configuration and iterators over tags and their test groups. Using
    /// this for storing test information and progress together.
    tag_iterators: Vec<TagIterator>,

    /// A lookup table of which tag group each evaluated tag was derived from.
    tag_derived_from: BTreeMap<String, Vec<String>>,

    /// Number of run tests that have failed, across all tags.
    tests_failed: usize,

    /// Maximum number of failed test cases to show. Any additional failed
    /// tests are hidden.
    tests_max_shown: usize,

    /// Deadline time. If we have reached or exceeded this timestamp, then we
    /// the total grading procedure has timed out.
    deadline_time: SystemTime,

    /// A flag indicating whether we have cleaned up the test procedure or not.
    /// Attempting to run a test case if this is set to true should result in a
    /// fatal error.
    cleaned_up: bool,

    /// Causes of overall test failures that should be reported back to the
    /// student.
    failure_cause: Option<FailureCause>,

    /// Settings for markdown output
    md_settings: RunnerMarkdownSettings,
}

/// Structure of collected results after a completed run.
pub struct TestRunnerCollectedResults {
    /// Files to generate in the shadow repository, provided as
    /// `(filename, contents)`. This will include information such as JSON
    /// formatted results, build outputs, etc.
    pub shadow_files: Vec<(PathBuf, String)>,

    /// The formatted Markdown to submit to the GitHub repository as a comment.
    pub gh_markdown: String,

    /// The status to mark the GitHub submission with.
    pub status: SubmissionStatusCode,
}

#[derive(Debug, Clone)]
pub enum FailureCause {
    /// The failure was caused by a total timeout, reached at the specified
    /// system time.
    TotalTimeout(SystemTime),

    /// An project build timed out.
    BuildTimeout,

    /// An individual test timed out.
    TestTimeout,

    /// An individual test breached the output length.
    OutputLengthBreached,

    /// The test runner was externally interrupted
    Interrupted,
}

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
        /// (path, mimetype)
        found_files: Vec<(String, String)>,
    },
    BuildFailed {
        message: Option<String>,
        code: Option<i32>,
        captured_stdout: Option<String>,
        captured_stderr: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub enum TestResult {
    TestOk,
    TestTimeout {
        captured_stdout: Option<String>,
        captured_stderr: Option<String>,
    },
    TestOutputLimitExceeded {
        limit: usize,
    },
    /// Provide None if details should be hidden from user
    TestFailed(Option<TestFailureDetails>),
}

/// The tuples are (received, expected)
#[derive(Debug, Clone)]
pub struct TestFailureDetails {
    message: Option<String>,
    mismatch_code: Option<(i32, i32)>,
    mismatch_stdout: Option<(String, Vec<String>)>,
    mismatch_stderr: Option<(String, Vec<String>)>,
    captured_code: Option<i32>,
    captured_stdout: Option<String>,
    captured_stderr: Option<String>,
    generated_asm: Option<String>,
}

impl Default for TestFailureDetails {
    fn default() -> Self {
        TestFailureDetails {
            message: None,
            mismatch_code: None,
            mismatch_stdout: None,
            mismatch_stderr: None,
            captured_code: None,
            captured_stdout: None,
            captured_stderr: None,
            generated_asm: None,
        }
    }
}

#[derive(Debug, Clone)]
struct TagIterator {
    /// The name of this tag, e.g. `hello` for the `#hello` tag.
    pub tag_name: String,

    /// Build configuration for this specific tag.
    pub build_conf: TagBuildConfig,

    /// Iterators for each respective test group contained within this tag.
    pub testgroup_iterators: Vec<TestGroupIterator>,

    next_testgroup: usize,

    /// Result of the build process
    build_result: Option<BuildResult>,
}

impl TagIterator {
    /// Creates a new tag iterator from a tag specification.
    fn new(tag: &Tag) -> Self {
        TagIterator {
            tag_name: tag.name.to_owned(),
            build_conf: tag.build.to_owned(),

            testgroup_iterators: tag
                .test_groups
                .iter()
                .map(|tg| TestGroupIterator::new(tg))
                .collect(),
            next_testgroup: 0,
            build_result: None,
        }
    }

    /// Returns the next test to run. Returns None if there is not a next test
    /// to run.
    fn peek(&self) -> Option<&Test> {
        self.testgroup_iterators
            .get(self.next_testgroup)
            .and_then(|tgi| tgi.peek())
    }

    /// Returns true if the solution for this project has been built.
    /// Irregardless of whether it was successfully built or not.
    fn has_built(&self) -> bool {
        self.build_result.is_some()
    }

    /// Returns true if the solution for this tag has successfully been built.
    fn is_build_ok(&self) -> bool {
        self.build_result.as_ref().map_or(false, |br| match br {
            BuildResult::BuildOk => true,
            _ => false,
        })
    }

    /// Adds the result from a build process.
    fn add_build_result(&mut self, res: BuildResult) -> Result<(), Error> {
        match self.build_result.replace(res) {
            Some(_) => Err(Error::from("Adding duplicate build results")),
            None => Ok(()),
        }
    }

    /// Adds the test result from a run.
    fn add_test_result(&mut self, res: TestResult) -> Result<(), Error> {
        self.testgroup_iterators
            .get_mut(self.next_testgroup)
            .ok_or(Error::from(
                "Internal Error: No test group iterator to add test result to",
            ))
            .and_then(|tgi| tgi.add_test_result(res))
    }

    /// Progress to the next test to run. Returns `true` if we could progress
    /// to a new test case. Returns `false` if we are at the end.
    fn next_test(&mut self) -> bool {
        while self.next_testgroup < self.testgroup_iterators.len() {
            let tg = self
                .testgroup_iterators
                .get_mut(self.next_testgroup)
                .unwrap();
            if tg.next_test() {
                return true;
            }
            // This test group is finished, progress to the next one.
            self.next_testgroup += 1;
        }
        return false;
    }
}

#[derive(Debug, Clone)]
struct TestGroupIterator {
    /// Metadata from the testgroup
    pub title: String,

    subgroup_iterators: Vec<TestGroupIterator>,
    next_subgroup: usize,
    next_test_idx: usize,
    has_checked_first_test: bool,
    // this is probably wasteful, but it works...
    tests: Vec<Test>,
    results: Vec<TestResult>,
}

impl TestGroupIterator {
    /// Creates a new iterator from a test group
    fn new(tg: &TestGroup) -> Self {
        TestGroupIterator {
            title: tg.title.to_owned(),
            subgroup_iterators: tg.subgroups.iter().map(|sg| Self::new(sg)).collect(),
            next_subgroup: 0,
            next_test_idx: 0,
            has_checked_first_test: false,
            tests: tg.tests.to_owned(),
            results: vec![],
        }
    }

    /// Returns the next test to run. Returns None if there is not a next test
    /// to run.
    fn peek(&self) -> Option<&Test> {
        if let Some(Some(t_opt)) = self
            .subgroup_iterators
            .get(self.next_subgroup)
            .map(|sg| sg.peek())
        {
            return Some(t_opt);
        }
        return self.tests.get(self.next_test_idx);
    }

    /// Adds the test result from a run
    fn add_test_result(&mut self, res: TestResult) -> Result<(), Error> {
        if let Some(sg) = self.subgroup_iterators.get_mut(self.next_subgroup) {
            return sg.add_test_result(res);
        }
        if self.results.len() != self.next_test_idx {
            return Err(Error::from(format!(
                "Internal error: Adding result to the wrong test case. At test {}, added result to {}.",
                self.next_test_idx,
                self.results.len()
            )));
        }
        self.results.push(res);
        Ok(())
    }

    /// Progresses to the next test case. Returns `true` if there is a new test
    /// to run. Returns `false` if we are at the end and there are no more
    /// tests to run for this tag group.
    fn next_test(&mut self) -> bool {
        while self.next_subgroup < self.subgroup_iterators.len() {
            let subgroup = self.subgroup_iterators.get_mut(self.next_subgroup).unwrap();
            if subgroup.next_test() {
                return true;
            }
            self.next_subgroup += 1;
        }

        // We need this check so that we do not progress beyond the first test
        // (would ideally like if we started at next_test_idx = -1 ...)
        if !self.has_checked_first_test {
            self.has_checked_first_test = true;
            return self.next_test_idx < self.tests.len();
        }

        if self.next_test_idx < self.tests.len() {
            self.next_test_idx += 1;
            return self.next_test_idx < self.tests.len();
        }

        return false;
    }
}

impl TestRunnerHandle {
    /// Creates a new handle, or returns an error message to be shown to the
    /// user. Internal error messages should be presented as log messages only,
    /// using map_err or inspect_err.
    pub fn new(settings: &Settings, sub: &Submission, runner_id: i32) -> Result<Self, String> {
        let tests = Tests::load(&settings.runner.test_config).map_err(|e| {
            log::error!("Could not load test configuration: {e}");
            ERRMSG_INTERNAL_ERROR.to_string()
        })?;

        // Collect the tags to run
        let mut tag_derived_from: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut tag_iterators: Vec<TagIterator> = vec![];
        for t in sub.grading_tags.split(";") {
            match tests.tag_groups.get(t) {
                Some(vec_tag) => {
                    for tag in vec_tag {
                        match tag_derived_from.get_mut(&tag.name) {
                            Some(derivs) => {
                                derivs.push(t.to_owned());
                            }
                            None => {
                                tag_iterators.push(TagIterator::new(tag));
                                tag_derived_from.insert(tag.name.to_owned(), vec![t.to_owned()]);
                            }
                        }
                    }
                }
                None => {
                    log::info!("Received invalid tag {t}");
                    // Format the error message on a GitHub Markdown friendly format
                    let mut direct_tags: Vec<String> = vec![];
                    let mut alias_tags: BTreeMap<String, Vec<String>> = BTreeMap::new();
                    for (k, tags) in tests.tag_groups.iter() {
                        if tags.len() == 1 && tags.get(0).map_or(false, |tag| tag.name == *k) {
                            direct_tags.push(k.to_owned());
                        } else {
                            alias_tags.insert(
                                k.to_owned(),
                                tags.iter().map(|tag| tag.name.to_owned()).collect(),
                            );
                        }
                    }
                    let mut md_string: String = format!("Unknown tag: `{t}`");
                    if direct_tags.len() > 0 {
                        md_string.push_str("\n\n### Known grading tags\n\n");
                        md_string
                            .push_str(&direct_tags.iter().map(|k| format!("* `{}`", k)).join("\n"));
                    }
                    if alias_tags.len() > 0 {
                        md_string.push_str("\n\n### Known tag groups\n\n");
                        md_string.push_str("| Group Name | Contained Grading Tags |\n");
                        md_string.push_str("| ---------- | ---------------------- |\n");
                        for (k, tagnames) in alias_tags.iter() {
                            md_string.push_str(&format!(
                                "| `{k}` | {} |\n",
                                tagnames.iter().map(|s| format!("`{s}`")).join(", ")
                            ));
                        }
                        md_string.push_str("\n"); // important with double LF after table
                    }
                    return Err(md_string);
                }
            }
        }

        // Create the workspace temp dir. We want this to persist, and do the
        // cleanup ourselves.
        let workspace_dir = path_absolute_join(
            &settings.runner.workspace_dir,
            format!("runner{runner_id}_{:08x}", rand::random::<u32>()),
        )
        .map_err(|e| {
            log::error!("Could not join workspace_dir path: {e}");
            ERRMSG_INTERNAL_ERROR.to_string()
        })?;

        let repo_dir = path_absolute_join(&workspace_dir, "repo").map_err(|e| {
            log::error!("Could not join repo_dir path: {e}");
            ERRMSG_INTERNAL_ERROR.to_string()
        })?;

        match std::fs::exists(&workspace_dir) {
            // Expected case
            Ok(false) => {}
            Ok(true) => {
                log::error!("Collision in workspace_dir: {workspace_dir}");
                return Err(ERRMSG_INTERNAL_ERROR.to_string());
            }
            Err(e) => {
                log::error!("Could not check for existence in filesystem: {e}");
                return Err(ERRMSG_INTERNAL_ERROR.to_string());
            }
        }

        std::fs::create_dir_all(&workspace_dir).map_err(|e| {
            log::error!("Could not create workspace_dir: {e}");
            ERRMSG_INTERNAL_ERROR.to_string()
        })?;

        // After this point we need to make sure that the workspace_dir is
        // deleted when the TestRunnerHandle is dropped.

        // A way to check out a specific commit, without the cloning the whole history
        let gitcmd_settings = SyscommandSettings {
            expected_code: Some(0),
            ..Default::default()
        };
        std::fs::create_dir_all(&repo_dir)
            .map_err(Error::from)
            .and_then(|_| {
                syscommand_timeout(
                    &["git", "-C", &repo_dir, "init"],
                    gitcmd_settings.to_owned(),
                )
            })
            .and_then(|_| {
                syscommand_timeout(
                    &[
                        "git",
                        "-C",
                        &repo_dir,
                        "remote",
                        "add",
                        "origin",
                        &format!(
                            "git@{}:{}/{}.git",
                            &sub.github_address, &sub.github_org, &sub.github_repo,
                        ),
                    ],
                    gitcmd_settings.to_owned(),
                )
            })
            .and_then(|_| {
                syscommand_timeout(
                    &[
                        "git",
                        "-C",
                        &repo_dir,
                        "fetch",
                        "--depth",
                        "1",
                        "origin",
                        &sub.github_commit,
                    ],
                    gitcmd_settings.to_owned(),
                )
            })
            .and_then(|_| {
                syscommand_timeout(
                    &["git", "-C", &repo_dir, "checkout", "FETCH_HEAD"],
                    gitcmd_settings.to_owned(),
                )
            })
            .map_err(|e| {
                log::error!("Error cloning repository {}: {e}", &sub.github_repo);
                std::fs::remove_dir_all(&workspace_dir)
                    .unwrap_or_else(|e| log::error!("Could not clean up workspace_dir: {e}"));
                ERRMSG_INTERNAL_ERROR.to_string()
            })?;

        let deadline_time = SystemTime::now()
            .checked_add(Duration::from_secs(tests.default.timeout_total.into()))
            .ok_or_else(|| {
                std::fs::remove_dir_all(&workspace_dir)
                    .unwrap_or_else(|e| log::error!("Could not clean up workspace_dir: {e}"));
                log::error!("Internal error setting deadline date.");
                ERRMSG_INTERNAL_ERROR.to_string()
            })?;

        Ok(TestRunnerHandle {
            podman_image: settings.runner.podman_image.clone(),
            podman_container_name: format!("id2202_runner{}", runner_id),
            podman_network_name: format!("{}{}", settings.runner.podman_network_prefix, runner_id),
            mount_repo: settings.runner.mount_repo.clone(),
            mount_tests: settings.runner.mount_tests.clone(),
            max_output: tests.default.max_output,
            truncate_len: tests.default.truncate_len,
            workspace: workspace_dir,
            submission_id: sub.id,
            repo_dir: repo_dir,
            repo_commit: sub.github_commit.to_owned(),
            next_tag_index: 0,
            tag_iterators: tag_iterators,
            tag_derived_from: tag_derived_from,
            tests_failed: 0,
            tests_max_shown: tests.default.shown_failures,
            deadline_time: deadline_time,
            cleaned_up: false,
            failure_cause: None,
            md_settings: settings.runner.md_settings.clone(),
        })
    }

    /// Check whether it is possible to run more tests. This will also return
    /// true if the runner has performed a cleanup.
    pub fn is_finished(&self) -> bool {
        return (self.next_tag_index >= self.tag_iterators.len())
            || self.cleaned_up
            || self.failure_cause.is_some();
    }

    /// Sets this test runner as finished, irregardless if it has finished or
    /// not. This might be due to an external error.
    pub fn set_as_finished(&mut self) -> () {
        self.failure_cause = Some(FailureCause::Interrupted);
        self.next_tag_index = self.tag_iterators.len();
    }

    /// Performs cleanup after grading. Removes any created containers and
    /// directories. After this has been called, it is no longer possible to
    /// run tests.
    pub fn cleanup(&mut self) -> () {
        log::debug!("Performing cleanup");
        if self.cleaned_up {
            log::warn!("Attempting to perform cleanup twice.");
            return;
        }

        let running_containers = podman::ps_names().unwrap_or_else(|e| {
            log::error!("Could not list running podman containers: {e}");
            vec![]
        });
        if running_containers.contains(&self.podman_container_name) {
            podman::force_rm(&self.podman_container_name).unwrap_or_else(|e| {
                log::error!("Error removing container from a previous run: {e}")
            });
        }

        log::debug!("Removing fs artifacts");
        std::fs::remove_dir_all(&self.workspace).unwrap_or_else(|e| {
            log::error!("Could not remove TestRunnerHandle workspace dir: {e}")
        });

        self.cleaned_up = true;
    }

    // Helper functions for `run_next`, to avoid rust's borrow checker
    fn next_test(&mut self) -> Result<bool, Error> {
        self.tag_iterators
            .get_mut(self.next_tag_index)
            .map(|tag| tag.next_test())
            .ok_or(Error::from("No tag to run, cannot progress"))
    }
    fn add_test_result(&mut self, res: TestResult) -> Result<(), Error> {
        use TestResult::{TestFailed, TestOutputLimitExceeded, TestTimeout};
        let test_to_add = match res {
            TestFailed(details) => {
                self.tests_failed += 1;
                if self.tests_failed > self.tests_max_shown {
                    TestFailed(None)
                } else {
                    TestFailed(details)
                }
            }
            TestTimeout { .. } => {
                if self.failure_cause.is_none() {
                    self.failure_cause = Some(FailureCause::TestTimeout);
                }
                res
            }
            TestOutputLimitExceeded { .. } => {
                if self.failure_cause.is_none() {
                    self.failure_cause = Some(FailureCause::OutputLengthBreached);
                }
                res
            }
            _ => res,
        };
        self.tag_iterators
            .get_mut(self.next_tag_index)
            .ok_or(Error::from("No tag to run, cannot add result"))
            .and_then(|tag| tag.add_test_result(test_to_add))
    }

    /// Run the next part of the testing runner process.
    ///
    /// This is the main aspect of the TestRunnerHandle. The owner of this
    /// handle should call this function in a loop until `is_finished`
    /// returns `true`. This can be seen as a form of small-step semantics.
    ///
    /// If this function returns an error, that is something that has gone
    /// wrong with the runner itself, not with a test case. If a test case
    /// could be successfully run (even if it timed out or otherwise failed),
    /// then this will return an Ok with a unit value. An Err return value
    /// indicates that something has gone wrong with the grading process itself
    /// and that the TestRunnerHandle must stop.
    pub fn run_next(&mut self) -> Result<(), Error> {
        use TestResult::{TestFailed, TestOk};

        if self.cleaned_up {
            return Err(Error::from(
                "Fatal: Attempting run a test case after performing cleanup.",
            ));
        }

        if SystemTime::now() >= self.deadline_time {
            log::info!("Grading procedure timed out.");
            self.failure_cause = Some(FailureCause::TotalTimeout(SystemTime::now()));
            return Ok(());
        }

        // Get the current tag to run.
        let tag: &mut TagIterator = self
            .tag_iterators
            .get_mut(self.next_tag_index)
            .ok_or(Error::from("No more tests to run"))?;

        let build_dir: String = path_absolute_join(&self.workspace, "build")?;
        let tests_dir: String = path_absolute_join(&self.workspace, "tests")?;

        if !tag.has_built() {
            log::info!(
                "Building project for tag \"{}\" (src: {})",
                tag.tag_name,
                tag.build_conf.srcdir
            );

            let running_containers = podman::ps_names()?;
            if running_containers.contains(&self.podman_container_name) {
                log::warn!("Removing dangling image from previous run");
                podman::force_rm(&self.podman_container_name)?;
            }

            if std::fs::exists(&build_dir)? {
                // Remove the old build dir
                std::fs::remove_dir_all(&build_dir)?;
            }
            if !std::fs::exists(&tests_dir)? {
                // Ensure that the test directory exists
                std::fs::create_dir_all(&tests_dir)?;
            }

            // Copy the solution directory to the <workspace>/build
            let solution_dir: String = path_absolute_join(&self.repo_dir, &tag.build_conf.srcdir)?;

            if !std::fs::exists(&solution_dir)? {
                tag.add_build_result(BuildResult::BuildSourceNotFound {
                    expected_dir: tag.build_conf.srcdir.to_owned(),
                })?;
                // Should not even try to build the project here, just move on.
                tag.next_test();
                return Ok(());
            }

            dircpy::copy_dir(&solution_dir, &build_dir)?;

            // (Path, mime output)
            let mut forbidden_files: Vec<(String, String)> = vec![];
            fn recur_scandir(
                forbidden_files: &mut Vec<(String, String)>,
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
                        .map_err(|oss| Error::from(format!("Invalid utf-8 filename: {oss:?}")))?;
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
                            forbidden_files.push((path, mimetype));
                        }
                    }
                }
                Ok(())
            }
            if tag.build_conf.prohibit_binary_files {
                log::debug!("Checking for prohibited files.");
                recur_scandir(
                    &mut forbidden_files,
                    &tag.build_conf.allowed_binary_files,
                    &tag.build_conf.allowed_binary_mimetypes,
                    PathBuf::from(&build_dir),
                    "".to_string(),
                )
                .inspect_err(|e| log::error!("Error when scanning for prohibited files: {e}"))?;
            }
            if forbidden_files.len() > 0 {
                tag.add_build_result(BuildResult::BuildProhibitedFiles {
                    found_files: forbidden_files,
                })?;
                // Should not even try to build the project here, just move on.
                tag.next_test();
                return Ok(());
            }

            log::debug!("Starting podman container");
            podman::start_container(&podman::ContainerOptions {
                image: self.podman_image.to_owned(),
                container_name: self.podman_container_name.to_owned(),
                network_name: self.podman_network_name.to_owned(),
                mounts: vec![
                    (
                        build_dir.to_owned(),
                        self.mount_repo.to_owned(),
                        "ro,z".to_string(),
                    ),
                    (
                        tests_dir.to_owned(),
                        self.mount_tests.to_owned(),
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
                    return Err(Error::from("Container would not start after 10 attempts."));
                }
                for ps_output in podman::ps()?.iter() {
                    if ps_output.names.contains(&self.podman_container_name)
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
                &self.podman_container_name,
                &["test", "!", "-d", "/root/graded_solution"],
            )?;

            // Now copy the solution to the root repository
            podman::exec(
                &self.podman_container_name,
                &["cp", "-r", &self.mount_repo, "/root/graded_solution"],
            )?;

            let mut build_cmd: Vec<&str> = vec![
                "podman",
                "exec",
                "-w",
                "/root/graded_solution",
                &self.podman_container_name,
            ];
            build_cmd.extend(tag.build_conf.cmd.iter().map(String::as_str));

            log::info!("Starting build {build_cmd:?}");

            match syscommand_timeout(
                build_cmd.as_slice(),
                SyscommandSettings {
                    max_stdout_length: Some(self.max_output),
                    max_stderr_length: Some(self.max_output),
                    timeout: Duration::from_secs(tag.build_conf.timeout.into()),
                    ..Default::default()
                },
            ) {
                Ok(output) => {
                    if output.code == 0 {
                        tag.add_build_result(BuildResult::BuildOk)?;
                    } else {
                        tag.add_build_result(BuildResult::BuildFailed {
                            message: None,
                            code: Some(output.code),
                            captured_stdout: Some(output.stdout),
                            captured_stderr: Some(output.stderr),
                        })?;
                    }
                }
                Err(Error::SyscommandTimeoutError { stdout, stderr }) => {
                    tag.add_build_result(BuildResult::BuildTimeout {
                        timeout: tag.build_conf.timeout,
                        captured_stdout: stdout,
                        captured_stderr: stderr,
                    })?;
                    self.failure_cause = Some(FailureCause::BuildTimeout);
                }
                Err(Error::SyscommandOutputLimitExceededError(limit)) => {
                    tag.add_build_result(BuildResult::BuildOutputLimitExceeded { limit: limit })?;
                    self.failure_cause = Some(FailureCause::OutputLengthBreached);
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
                    &self.podman_network_name,
                    &self.podman_container_name,
                ],
                SyscommandSettings {
                    expected_code: Some(0),
                    ..Default::default()
                },
            )?;

            log::info!("Proceeding to run test cases.");

            // Now we are ready to progress to the first test
            tag.next_test();

            return Ok(());
        }

        // If we have built but the build is not OK, then we need to move on to
        // the next tag to be graded.
        if !tag.is_build_ok() {
            self.next_tag_index += 1;
            return Ok(());
        }

        // Doing the cloning here because of rusts mutability checker.
        let test: Test = {
            match tag.peek() {
                Some(t) => t,
                None => {
                    // No more test to run for this tag. Move to the next tag
                    // in the group.
                    self.next_tag_index += 1;
                    return Ok(());
                }
            }
            .clone()
        };

        let outside_test_file = path_absolute_join(&tests_dir, "test.in")?;
        let inside_test_file = path_absolute_join(&self.mount_tests, "test.in")?;

        match &test.kind {
            Testkind::GenASMAndRun(base_config) => {
                let conf = &base_config.base;
                // The difference between Testkind::Run is that we immediately
                // need to return on a failed test here. So we do the
                // `tag.next_test()` and Ok(()) for each place where we need to
                // exit. This is not pretty, but it works.

                let run_output = self.run_solution(RunSolutionArgs {
                    bin: conf.bin.to_owned(),
                    cmdargs: conf.args.to_owned(),
                    stdin: if conf.ignore_stdin {
                        None
                    } else {
                        Some(conf.stdin.to_owned())
                    },
                    capture_stdout: true,
                    expect_code: Some(conf.code),
                    stderr_expect_alternatives: if conf.ignore_stderr {
                        None
                    } else {
                        Some(vec![conf.stderr.to_owned()])
                    },
                    stderr_trim: conf.trim_stderr,
                    stderr_rm_whitespace: conf.strip_whitespace_stderr,
                    infile_path: Some(base_config.input_file.to_owned()),
                    host_infile: outside_test_file.to_owned(),
                    container_infile: inside_test_file.to_owned(),
                    timeout: test.timeout.into(),
                    ..Default::default()
                })?;

                if run_output.failed {
                    self.next_test()?;
                    return Ok(());
                }

                let grade_output = self.grade_asm_output(GradeASMArgs {
                    tests_dir: tests_dir,
                    asm_contents: run_output.stdout,
                    asm_cmd: conf.assemble_cmd.to_owned(),
                    asm_code: Some(conf.assemble_code),
                    compile_cmd: conf.compile_cmd.to_owned(),
                    compile_code: Some(conf.compile_code),
                    run_cmd: conf.run_cmd.to_owned(),
                    stdin: if conf.run_ignore_stdin {
                        None
                    } else {
                        Some(conf.stdin.to_owned())
                    },
                    expect_code: Some(conf.run_code),
                    stdout_expect: if conf.run_ignore_stdout {
                        None
                    } else {
                        Some(conf.run_stdout.to_owned())
                    },
                    stdout_trim: conf.run_trim_stdout,
                    stdout_rm_whitespace: conf.run_strip_whitespace_stdout,
                    stderr_expect: if conf.run_ignore_stderr {
                        None
                    } else {
                        Some(conf.run_stderr.to_owned())
                    },
                    stderr_trim: conf.run_trim_stderr,
                    stderr_rm_whitespace: conf.run_strip_whitespace_stderr,
                    timeout: test.timeout,
                })?;

                if grade_output.failed {
                    self.next_test()?;
                    return Ok(());
                }
            }
            Testkind::Run(base_config) => {
                let conf = &base_config.base;

                let run_output = self.run_solution(RunSolutionArgs {
                    bin: conf.bin.to_owned(),
                    cmdargs: conf.args.to_owned(),
                    stdin: if conf.ignore_stdin {
                        None
                    } else {
                        Some(conf.stdin.to_owned())
                    },
                    capture_stdout: false,
                    expect_code: Some(conf.code),
                    stdout_expect_alternatives: if conf.ignore_stdout {
                        None
                    } else {
                        let mut alternatives = vec![conf.stdout.to_owned()];
                        alternatives.extend_from_slice(conf.stdout_alternatives.as_slice());
                        Some(alternatives)
                    },
                    stdout_trim: conf.trim_stdout,
                    stdout_rm_whitespace: conf.strip_whitespace_stdout,
                    stderr_expect_alternatives: if conf.ignore_stderr {
                        None
                    } else {
                        let mut alternatives = vec![conf.stderr.to_owned()];
                        alternatives.extend_from_slice(conf.stderr_alternatives.as_slice());
                        Some(alternatives)
                    },
                    stderr_trim: conf.trim_stderr,
                    stderr_rm_whitespace: conf.strip_whitespace_stderr,
                    infile_path: base_config.input_file.to_owned(),
                    host_infile: outside_test_file.to_owned(),
                    container_infile: inside_test_file.to_owned(),
                    timeout: test.timeout.into(),
                })?;

                if run_output.failed {
                    self.next_test()?;
                    return Ok(());
                }
            }
            Testkind::CheckFileExists(conf) => {
                // In this case, we only check that the file exists
                let check_path = path_absolute_join(&build_dir, &conf.path)?;
                if !std::fs::exists(&check_path)? {
                    log::error!("File not found at path {check_path}");
                    self.add_test_result(TestFailed(Some(TestFailureDetails {
                        message: Some("File not found.".to_string()),
                        ..Default::default()
                    })))?;
                    return Ok(());
                }
                if !conf.ignore_mimetype {
                    let mimetype = utils::mimetype(&check_path)
                        .inspect_err(|e| log::error!("Could not check file {check_path}: {e}"))?;
                    if !mimetype.starts_with(&conf.mimetype_prefix) {
                        self.add_test_result(TestFailed(Some(TestFailureDetails {
                            message: Some(format!("Invalid MIME type {mimetype}.")),
                            ..Default::default()
                        })))?;
                        return Ok(());
                    }
                }
                // Test OK, file exists and is of acceptable type
            }
        }

        // We reached the end without failure. Mark the test as OK
        self.add_test_result(TestOk)?;

        self.next_test()?;
        return Ok(());
    }

    /// Collects the results from this runner instance.
    ///
    /// See helper functions `recursively_fetch` and
    /// `add_md_test_details`.
    pub fn collect_results(&self) -> TestRunnerCollectedResults {
        let mut shadow_files: Vec<(PathBuf, String)> = vec![];
        let mut status: Option<SubmissionStatusCode> = None;
        let mut gh_markdown: String = format!("# Submission Results");

        match self.failure_cause.as_ref() {
            Some(FailureCause::BuildTimeout) => {
                gh_markdown
                    .push_str("\n\n_(Grading process was interrupted due to a build timeout.)_");
                status = Some(SubmissionStatusCode::BuildTimedOut);
            }
            Some(FailureCause::TestTimeout) => {
                gh_markdown
                    .push_str("\n\n_(Grading process was interrupted due to a test timeout.)_");
                status = Some(SubmissionStatusCode::TestCasesTimedOut);
            }
            Some(FailureCause::OutputLengthBreached) => {
                gh_markdown.push_str(
                    "\n\n_(Grading process was interrupted due to an exceeded output length.)_",
                );
                status = Some(SubmissionStatusCode::TestCasesFailed);
            }
            Some(FailureCause::TotalTimeout(t)) => {
                gh_markdown.push_str(&format!(
                    "\n\nGrading process timed out at {}.",
                    systemtime_to_utc_string(t).unwrap_or("...".to_string())
                ));
                status = Some(SubmissionStatusCode::TestCasesTimedOut);
            }
            Some(FailureCause::Interrupted) => {
                gh_markdown
                    .push_str("\n\n_(Grading process was interrupted. Contact course staff.)_");
                status = Some(SubmissionStatusCode::AutograderFailure);
            }
            None => {}
        }

        gh_markdown.push_str("\n\nTests are grouped together into categories.");
        gh_markdown.push_str(" Each category contains a set of test cases that evaluate a specific aspect of your program.");

        gh_markdown.push_str(&format!(
            "\n\n * The symbol {} indicates that all the tests in the category passed.",
            self.md_settings.symbol_ok
        ));
        gh_markdown.push_str(&format!(
            "\n * The symbol {} indicates that not all tests were run in this category. This is usually due to a build error or a previous test timeout.",
            self.md_settings.symbol_skipped
        ));
        gh_markdown.push_str(&format!(
            "\n * The symbol {} indicates that at least one test in the category failed. In this case, you will see how many test cases failed, along with some hints about what is tested in the category.",
            self.md_settings.symbol_failed
        ));

        gh_markdown.push_str(&format!(
            "\n\nAdditionally, for the first {} tests that fail, you will also get more detailed information after the main overview.",
            self.tests_max_shown
        ));

        let mut additional_md_details: Vec<String> = vec![];

        for tag in self.tag_iterators.iter() {
            gh_markdown.push_str(&format!("\n\n## Results for tag `{}`", tag.tag_name));

            // Check if the tag is derived from a differently named tag group
            let derivs: Vec<String> = self
                .tag_derived_from
                .get(&tag.tag_name)
                .unwrap_or(&vec![])
                .iter()
                .filter(|d| **d != tag.tag_name)
                .map(String::to_owned)
                .collect();
            if derivs.len() > 0 {
                gh_markdown.push_str(&format!(
                    "\n\n_(Derived from {})_",
                    derivs.iter().map(|s| format!("`{}`", s)).join(", ")
                ));
            }

            let mut tag_json = json::object! {
                commit: self.repo_commit.to_owned(),
                tag_name: tag.tag_name.to_owned(),
                ok: false,
            };

            match &tag.build_result {
                Some(BuildResult::BuildOk) => {
                    // Build is OK, fetch results from test cases

                    let mut tag_ok = true;
                    let mut tag_skipped = false;
                    let mut tgi_md_strs: Vec<String> = vec![];
                    let mut tgi_json_values: Vec<json::JsonValue> = vec![];
                    for tgi in tag.testgroup_iterators.iter() {
                        let (tgi_ok, tgi_skipped, md_str, json_details) =
                            self.recursively_fetch(tgi, 0, &mut additional_md_details);
                        tag_ok = tag_ok && tgi_ok;
                        tag_skipped = tag_skipped || tgi_skipped;
                        tgi_md_strs.push(md_str);
                        tgi_json_values.push(json_details);
                    }
                    tag_json["ok"] = tag_ok.into();
                    if tag_ok {
                        gh_markdown.push_str(&format!(
                            "\n\nAll test cases passed for this tag! {}\n",
                            self.md_settings.symbol_tagsuccess
                        ));
                    } else {
                        tag_json["reason"] = "test failed".into();
                        status = status.or(Some(SubmissionStatusCode::TestCasesFailed));
                        gh_markdown.push_str("\n\nSome test cases failed. See details below.\n");
                    }
                    tag_json["test_results"] = tgi_json_values.into();
                    for s in tgi_md_strs.iter() {
                        gh_markdown.push_str("\n");
                        gh_markdown.push_str(s);
                    }
                }
                Some(BuildResult::BuildSourceNotFound { expected_dir }) => {
                    gh_markdown.push_str(&format!(
                        "\n\n{} Build process failed. The source directory `{}` is absent in your repository.",
                        self.md_settings.symbol_build,
                        expected_dir
                    ));
                    tag_json["reason"] = "source directory not found".into();
                    status = status.or(Some(SubmissionStatusCode::BuildError));
                }
                Some(BuildResult::BuildFailed {
                    message,
                    code,
                    captured_stdout,
                    captured_stderr,
                }) => {
                    gh_markdown.push_str(&format!(
                        "\n\n{} Build process failed.",
                        self.md_settings.symbol_build
                    ));
                    if let Some(buildmsg) = message {
                        gh_markdown.push_str(" ");
                        gh_markdown.push_str(&buildmsg);
                    }
                    gh_markdown.push_str(&format!(
                        "\n\n**Source Directory:** `{}`",
                        tag.build_conf.srcdir
                    ));
                    gh_markdown.push_str(&format!(
                        "\n\n**Build command:** `{}`",
                        tag.build_conf.cmd.join(" ")
                    ));
                    if let Some(c) = code {
                        gh_markdown.push_str(&format!("\n\nExit code: `{c}`"));
                    }
                    if let Some(stdout) = captured_stdout {
                        gh_markdown.push_str(&format!(
                            "\n\n### Captured Standard Output\n\n```\n{stdout}\n```"
                        ));
                    }
                    if let Some(stderr) = captured_stderr {
                        gh_markdown.push_str(&format!(
                            "\n\n### Captured Standard Error\n\n```\n{stderr}\n```"
                        ));
                    }
                    tag_json["reason"] = "build failed".into();
                    status = status.or(Some(SubmissionStatusCode::BuildError));
                }
                Some(BuildResult::BuildProhibitedFiles { found_files }) => {
                    gh_markdown.push_str(&format!(
                        "\n\n{} Build failed due to unexpected non-text files in your solution directory. These files are:\n",
                        self.md_settings.symbol_build,
                    ));
                    for (path, mimetype) in found_files.iter() {
                        gh_markdown.push_str(&format!(
                            "\n * `{}` (Identified as MIME type `{}`)",
                            path, mimetype
                        ));
                    }
                    gh_markdown.push_str("\n\nPlease remove these files from your solution directory and make sure that your .gitignore is properly configured.");
                    tag_json["reason"] = "prohibited files".into();
                    status = status.or(Some(SubmissionStatusCode::BuildError));
                }
                Some(BuildResult::BuildTimeout {
                    timeout,
                    captured_stdout,
                    captured_stderr,
                }) => {
                    gh_markdown.push_str(&format!(
                        "\n\n{} Build process timed out after {timeout} seconds.",
                        self.md_settings.symbol_build
                    ));
                    if let Some(stdout) = captured_stdout {
                        gh_markdown.push_str(&format!(
                            "\n\n### Captured Standard Output\n\n```\n{stdout}\n```"
                        ));
                    }
                    if let Some(stderr) = captured_stderr {
                        gh_markdown.push_str(&format!(
                            "\n\n### Captured Standard Error\n\n```\n{stderr}\n```"
                        ));
                    }
                    tag_json["reason"] = "build timed out".into();
                    status = status.or(Some(SubmissionStatusCode::BuildTimedOut));
                }
                Some(BuildResult::BuildOutputLimitExceeded { limit }) => {
                    gh_markdown.push_str(&format!(
                        "\n\n{} Output limit of {limit} bytes exceeded when building project.",
                        self.md_settings.symbol_build
                    ));
                    tag_json["reason"] = "build output limit exceeded".into();
                    status = status.or(Some(SubmissionStatusCode::BuildError));
                }
                None => {
                    gh_markdown.push_str(&format!(
                        "\n\n{} Grading was interupted prior to building project.",
                        self.md_settings.symbol_skipped
                    ));
                    tag_json["reason"] = "no build status present".into();
                    status = status.or(Some(SubmissionStatusCode::BuildError));
                }
            }

            shadow_files.push((
                PathBuf::from(format!("{}.results.json", tag.tag_name)),
                tag_json.to_string(),
            ));
        }

        for (i, detail) in additional_md_details.iter().enumerate() {
            gh_markdown.push_str("\n\n");
            gh_markdown.push_str(&format!("<details id=\"detail-summary-{}\">\n", i + 1));
            gh_markdown.push_str(&format!("<summary>Detail {}</summary>\n\n", i + 1));
            gh_markdown.push_str(detail);
            gh_markdown.push_str("\n\n</details>");
        }

        TestRunnerCollectedResults {
            shadow_files: shadow_files,
            gh_markdown: gh_markdown,
            status: status.unwrap_or(SubmissionStatusCode::Success),
        }
    }
}

//  _   _      _                   _____                 _   _
// | | | | ___| |_ __   ___ _ __  |  ___|   _ _ __   ___| |_(_) ___  _ __  ___
// | |_| |/ _ \ | '_ \ / _ \ '__| | |_ | | | | '_ \ / __| __| |/ _ \| '_ \/ __|
// |  _  |  __/ | |_) |  __/ |    |  _|| |_| | | | | (__| |_| | (_) | | | \__ \
// |_| |_|\___|_| .__/ \___|_|    |_|   \__,_|_| |_|\___|\__|_|\___/|_| |_|___/
//              |_|

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
        .map_err(|e| Error::from(format!("Internal error removing whitespace: {e}")))
    } else if trim {
        Ok(s.trim_ascii().to_string())
    } else {
        Ok(s.to_string())
    }
}

struct RunSolutionArgs {
    bin: String,
    cmdargs: Vec<String>,
    stdin: Option<String>,
    capture_stdout: bool,
    expect_code: Option<i32>,
    stdout_expect_alternatives: Option<Vec<String>>,
    stdout_trim: bool,
    stdout_rm_whitespace: bool,
    stderr_expect_alternatives: Option<Vec<String>>,
    stderr_trim: bool,
    stderr_rm_whitespace: bool,
    infile_path: Option<String>,
    host_infile: String,
    container_infile: String,
    timeout: u32,
}

impl Default for RunSolutionArgs {
    fn default() -> Self {
        RunSolutionArgs {
            bin: "".to_string(),
            cmdargs: vec![],
            stdin: None,
            capture_stdout: false,
            expect_code: None,
            stdout_expect_alternatives: None,
            stdout_trim: false,
            stdout_rm_whitespace: false,
            stderr_expect_alternatives: None,
            stderr_trim: false,
            stderr_rm_whitespace: false,
            infile_path: None,
            host_infile: "".to_string(),
            container_infile: "".to_string(),
            timeout: 60,
        }
    }
}

struct RunSolutionOutput {
    failed: bool,
    stdout: String,
}

impl TestRunnerHandle {
    /// Common function to run the compiled solution for different test kinds,
    /// performing checks such that the output is what would be expected as
    /// well.
    ///
    /// This will also update the state of the TestRunnerHandle by adding
    /// results for each run test.
    fn run_solution(&mut self, args: RunSolutionArgs) -> Result<RunSolutionOutput, Error> {
        use TestResult::{TestFailed, TestOutputLimitExceeded, TestTimeout};

        let executable = format!("./{}", args.bin);
        let mut test_cmd = vec!["podman", "exec", "-w", "/root/graded_solution"];
        if args.stdin.is_some() {
            // This is needed for podman to capture stdin
            test_cmd.push("-i");
        }
        test_cmd.push(&self.podman_container_name);
        test_cmd.push(&executable);
        test_cmd.extend(args.cmdargs.iter().map(String::as_str));
        if let Some(infile) = &args.infile_path {
            // Copy file to the tests_dir and add it to the command
            std::fs::copy(infile, &args.host_infile).inspect_err(|e| {
                log::error!(
                    "Could not copy input file {} to {}: {e}",
                    infile,
                    args.host_infile
                )
            })?;
            test_cmd.push(&args.container_infile);
        }

        let res = syscommand_timeout(
            test_cmd.as_slice(),
            SyscommandSettings {
                stdin: args.stdin.map(String::from),
                max_stdout_length: Some(self.max_output),
                max_stderr_length: Some(self.max_output),
                timeout: Duration::from_secs(args.timeout.into()),
                ..Default::default()
            },
        );
        if let Some(_) = &args.infile_path {
            // Remove the file that was used in the test case
            std::fs::remove_file(&args.host_infile)
                .unwrap_or_else(|e| log::error!("Could not remove input file: {e}"));
        }

        match res {
            Ok(output) => {
                // Check the expected statuses
                let mut msgs: Vec<String> = vec![];

                let code_status = args.expect_code.and_then(|c| {
                    if output.code != c {
                        Some((output.code, c))
                    } else {
                        None
                    }
                });
                let stdout_status = match args.stdout_expect_alternatives {
                    Some(expect_stdout_alternatives) => {
                        let mut found_match = false;
                        for alt_stdout in expect_stdout_alternatives.iter() {
                            found_match |= treat_output(
                                &output.stdout,
                                args.stdout_trim,
                                args.stdout_rm_whitespace,
                            )? == treat_output(
                                &alt_stdout,
                                args.stdout_trim,
                                args.stdout_rm_whitespace,
                            )?;
                        }
                        if !found_match {
                            if args.stdout_rm_whitespace {
                                msgs.push(
                                    "Whitespaces are ignored on standard output.".to_string(),
                                );
                            } else if args.stdout_trim {
                                msgs.push("Leading and trailing whitespaces are ignored on standard output.".to_string());
                            }
                            Some((
                                output.stdout.to_owned(),
                                expect_stdout_alternatives.to_owned(),
                            ))
                        } else {
                            None
                        }
                    }
                    None => None,
                };
                let stderr_status = match args.stderr_expect_alternatives {
                    Some(expect_stderr_alternatives) => {
                        let mut found_match = false;
                        for alt_stderr in expect_stderr_alternatives.iter() {
                            found_match |= treat_output(
                                &output.stderr,
                                args.stderr_trim,
                                args.stderr_rm_whitespace,
                            )? == treat_output(
                                &alt_stderr,
                                args.stderr_trim,
                                args.stderr_rm_whitespace,
                            )?;
                        }
                        if !found_match {
                            if args.stderr_rm_whitespace {
                                msgs.push("Whitespaces are ignored on standard error.".to_string());
                            } else if args.stderr_trim {
                                msgs.push("Leading and trailing whitespaces are ignored on standard error.".to_string());
                            }
                            Some((
                                output.stderr.to_owned(),
                                expect_stderr_alternatives.to_owned(),
                            ))
                        } else {
                            None
                        }
                    }
                    None => None,
                };

                match (code_status, stdout_status, stderr_status) {
                    (None, None, None) => {
                        return Ok(RunSolutionOutput {
                            failed: false,
                            stdout: if args.capture_stdout {
                                output.stdout
                            } else {
                                "".to_string()
                            },
                        });
                    }
                    (c, out, err) => {
                        self.add_test_result(TestFailed(Some(TestFailureDetails {
                            message: if msgs.len() == 0 {
                                None
                            } else {
                                Some(format!("(NOTE: {})", msgs.iter().join(" ")))
                            },
                            mismatch_code: c,
                            mismatch_stdout: out.to_owned(),
                            mismatch_stderr: err.to_owned(),
                            captured_code: if c.is_none() { Some(output.code) } else { None },
                            captured_stdout: if (&out).is_none() {
                                Some(output.stdout.to_owned())
                            } else {
                                None
                            },
                            captured_stderr: if (&err).is_none() {
                                Some(output.stderr.to_owned())
                            } else {
                                None
                            },
                            generated_asm: None,
                        })))?;
                        return Ok(RunSolutionOutput {
                            failed: true,
                            stdout: "".to_string(),
                        });
                    }
                }
            }
            Err(Error::SyscommandTimeoutError { stdout, stderr }) => {
                self.add_test_result(TestTimeout {
                    captured_stdout: stdout,
                    captured_stderr: stderr,
                })?;
                return Ok(RunSolutionOutput {
                    failed: true,
                    stdout: "".to_string(),
                });
            }
            Err(Error::SyscommandOutputLimitExceededError(limit)) => {
                self.add_test_result(TestOutputLimitExceeded { limit: limit })?;
                return Ok(RunSolutionOutput {
                    failed: true,
                    stdout: "".to_string(),
                });
            }
            Err(e) => {
                log::error!("Unknown error happened when running test case in a container: {e}");
                return Err(e);
            }
        }
    }
}

struct GradeASMArgs {
    tests_dir: String,
    asm_contents: String,
    asm_cmd: Vec<String>,
    asm_code: Option<i32>,
    compile_cmd: Vec<String>,
    compile_code: Option<i32>,
    run_cmd: Vec<String>,
    stdin: Option<String>,
    expect_code: Option<i32>,
    stdout_expect: Option<String>,
    stdout_trim: bool,
    stdout_rm_whitespace: bool,
    stderr_expect: Option<String>,
    stderr_trim: bool,
    stderr_rm_whitespace: bool,
    timeout: u32,
}

struct GradeASMOutput {
    failed: bool,
}

impl TestRunnerHandle {
    /// Common function to run the compiled solution for different test kinds,
    /// performing checks such that the output is what would be expected as
    /// well.
    ///
    /// This will also update the state of the TestRunnerHandle by adding
    /// results for each run test.
    fn grade_asm_output(&mut self, args: GradeASMArgs) -> Result<GradeASMOutput, Error> {
        use TestResult::{TestFailed, TestOutputLimitExceeded, TestTimeout};

        // Set up the /tmp/grading dir and write the asm program there

        let outside_asm_path = path_absolute_join(&args.tests_dir, "gen.asm")?;
        let inside_asm_path = path_absolute_join(&self.mount_tests, "gen.asm")?;
        // Open the file in a separate scope to ensure that it is closed
        {
            let mut asm_f = std::fs::File::create(&outside_asm_path)
                .inspect_err(|e| log::error!("Cannot create ASM file {outside_asm_path}: {e}"))?;
            asm_f.write(args.asm_contents.as_bytes())?;
            asm_f.flush()?;
        }

        syscommand_timeout(
            &[
                "podman",
                "exec",
                &self.podman_container_name,
                "bash",
                "-c",
                &format!("rm -rf /tmp/grading && mkdir -p /tmp/grading && cp {inside_asm_path} /tmp/grading/gen.asm"),
            ],
            SyscommandSettings {
                expected_code: Some(0),
                ..Default::default()
            },
        )?;
        std::fs::remove_file(&outside_asm_path)
            .inspect_err(|e| log::error!("Error removing ASM file {outside_asm_path}: {e}"))?;

        // Now write the generated assembly program to a path
        // (set up the NASM command separately to make sure that we
        // replace the template <ASM_FILE> with the true filename.)
        let mut nasm_cmd: Vec<&str> = vec![
            "podman",
            "exec",
            "-w",
            "/tmp/grading",
            &self.podman_container_name,
        ];
        nasm_cmd.extend(args.asm_cmd.iter().map(|s| {
            if s == "<ASM_FILE>" {
                "gen.asm"
            } else {
                s.as_str()
            }
        }));

        let nasm_settings = SyscommandSettings {
            expected_code: None,
            max_stdout_length: Some(self.max_output),
            max_stderr_length: Some(self.max_output),
            ..Default::default()
        };

        match syscommand_timeout(nasm_cmd.as_slice(), nasm_settings) {
            Ok(output) => {
                if let Some(c) = args.asm_code {
                    if output.code != c {
                        self.add_test_result(TestFailed(Some(TestFailureDetails {
                            message: Some("Failed to assemble the generated program.".to_string()),
                            mismatch_code: Some((output.code, c)),
                            captured_stdout: Some(output.stdout),
                            captured_stderr: Some(output.stderr),
                            generated_asm: Some(args.asm_contents.to_owned()),
                            ..Default::default()
                        })))?;
                        return Ok(GradeASMOutput { failed: true });
                    }
                }
            }
            Err(Error::SyscommandTimeoutError { stdout, stderr }) => {
                self.add_test_result(TestTimeout {
                    captured_stdout: stdout,
                    captured_stderr: stderr,
                })?;
                return Ok(GradeASMOutput { failed: true });
            }
            Err(Error::SyscommandOutputLimitExceededError(limit)) => {
                self.add_test_result(TestOutputLimitExceeded { limit: limit })?;
                return Ok(GradeASMOutput { failed: true });
            }
            Err(e) => {
                log::error!("Unknown error happened when running test case in a container: {e}");
                return Err(e);
            }
        }

        // Run the compilation step
        let mut compile_cmd: Vec<&str> = vec![
            "podman",
            "exec",
            "-w",
            "/tmp/grading",
            &self.podman_container_name,
        ];
        compile_cmd.extend(args.compile_cmd.iter().map(String::as_str));

        let compile_settings = SyscommandSettings {
            expected_code: None,
            max_stdout_length: Some(self.max_output),
            max_stderr_length: Some(self.max_output),
            ..Default::default()
        };

        match syscommand_timeout(compile_cmd.as_slice(), compile_settings) {
            Ok(output) => {
                if let Some(c) = args.compile_code {
                    if output.code != c {
                        self.add_test_result(TestFailed(Some(TestFailureDetails {
                            message: Some(
                                "Failed to compile the generated assembly program.".to_string(),
                            ),
                            mismatch_code: Some((output.code, c)),
                            captured_stdout: Some(output.stdout),
                            captured_stderr: Some(output.stderr),
                            generated_asm: Some(args.asm_contents.to_owned()),
                            ..Default::default()
                        })))?;
                        return Ok(GradeASMOutput { failed: true });
                    }
                }
            }
            Err(Error::SyscommandTimeoutError { stdout, stderr }) => {
                self.add_test_result(TestTimeout {
                    captured_stdout: stdout,
                    captured_stderr: stderr,
                })?;
                return Ok(GradeASMOutput { failed: true });
            }
            Err(Error::SyscommandOutputLimitExceededError(limit)) => {
                self.add_test_result(TestOutputLimitExceeded { limit: limit })?;
                return Ok(GradeASMOutput { failed: true });
            }
            Err(e) => {
                log::error!("Unknown error happened when running test case in a container: {e}");
                return Err(e);
            }
        }

        // Finally do the grading part
        let mut run_cmd: Vec<&str> = vec!["podman", "exec", "-w", "/tmp/grading"];
        if args.stdin.is_some() {
            run_cmd.push("-i");
        }
        run_cmd.push(&self.podman_container_name);
        run_cmd.extend(args.run_cmd.iter().map(String::as_str));

        match syscommand_timeout(
            run_cmd.as_slice(),
            SyscommandSettings {
                stdin: args.stdin,
                max_stdout_length: Some(self.max_output),
                max_stderr_length: Some(self.max_output),
                timeout: Duration::from_secs(args.timeout.into()),
                ..Default::default()
            },
        ) {
            Ok(output) => {
                let mut msgs: Vec<String> = vec![];

                let code_status = args.expect_code.and_then(|c| {
                    if output.code != c {
                        Some((output.code, c))
                    } else {
                        None
                    }
                });
                let stdout_status = match args.stdout_expect {
                    Some(expect_stdout) => {
                        if treat_output(
                            &output.stdout,
                            args.stdout_trim,
                            args.stdout_rm_whitespace,
                        )? != treat_output(
                            &expect_stdout,
                            args.stdout_trim,
                            args.stdout_rm_whitespace,
                        )? {
                            if args.stdout_rm_whitespace {
                                msgs.push(
                                    "Whitespaces are ignored on standard output.".to_string(),
                                );
                            } else if args.stdout_trim {
                                msgs.push("Leading and trailing whitespaces are ignored on standard output.".to_string());
                            }
                            Some((output.stdout.to_owned(), vec![expect_stdout.to_owned()]))
                        } else {
                            None
                        }
                    }
                    None => None,
                };
                let stderr_status = match args.stderr_expect {
                    Some(expect_stderr) => {
                        if treat_output(
                            &output.stderr,
                            args.stderr_trim,
                            args.stderr_rm_whitespace,
                        )? != treat_output(
                            &expect_stderr,
                            args.stderr_trim,
                            args.stderr_rm_whitespace,
                        )? {
                            if args.stderr_rm_whitespace {
                                msgs.push("Whitespaces are ignored on standard error.".to_string());
                            } else if args.stderr_trim {
                                msgs.push("Leading and trailing whitespaces are ignored on standard error.".to_string());
                            }
                            Some((output.stderr.to_owned(), vec![expect_stderr.to_owned()]))
                        } else {
                            None
                        }
                    }
                    None => None,
                };

                match (code_status, stdout_status, stderr_status) {
                    (None, None, None) => {
                        return Ok(GradeASMOutput { failed: false });
                    }
                    (c, out, err) => {
                        self.add_test_result(TestFailed(Some(TestFailureDetails {
                            message: if msgs.len() == 0 {
                                None
                            } else {
                                Some(format!("(NOTE: {})", msgs.iter().join(" ")))
                            },
                            mismatch_code: c,
                            mismatch_stdout: out.to_owned(),
                            mismatch_stderr: err.to_owned(),
                            captured_code: if c.is_none() { Some(output.code) } else { None },
                            captured_stdout: if (&out).is_none() {
                                Some(output.stdout.to_owned())
                            } else {
                                None
                            },
                            captured_stderr: if (&err).is_none() {
                                Some(output.stderr.to_owned())
                            } else {
                                None
                            },
                            generated_asm: Some(args.asm_contents.to_owned()),
                        })))?;
                        return Ok(GradeASMOutput { failed: true });
                    }
                }
            }
            Err(Error::SyscommandTimeoutError { stdout, stderr }) => {
                self.add_test_result(TestTimeout {
                    captured_stdout: stdout,
                    captured_stderr: stderr,
                })?;
                return Ok(GradeASMOutput { failed: true });
            }
            Err(Error::SyscommandOutputLimitExceededError(limit)) => {
                self.add_test_result(TestOutputLimitExceeded { limit: limit })?;
                return Ok(GradeASMOutput { failed: true });
            }
            Err(e) => {
                log::error!("Unknown error happened when running test case in a container: {e}");
                return Err(e);
            }
        }
    }

    /// Recursively fetches test result information for each test
    /// group. Constructs both JSON and Markdown summaries.
    ///
    /// Helper function for `TestRunner::collect_results`.
    ///
    /// Note: Indent by 2 for lists. E.g.
    /// ```
    ///  * A
    ///    * B
    ///    * B2
    ///      * C
    ///    * B3
    /// ```
    /// etc.
    fn recursively_fetch(
        &self,
        tgi: &TestGroupIterator,
        indent: usize,
        additional_md_details: &mut Vec<String>,
    ) -> (bool, bool, String, json::JsonValue) {
        //
        let mut tgi_ok = true;
        let mut tgi_skipped = false;
        let mut title_suffix: String = "".to_string();
        let mut json_details = json::object! {
            name: tgi.title.to_owned(),
        };
        let mut generated_md_details: Vec<usize> = vec![];
        if tgi.tests.len() > 0 {
            // Scan the results that are part of this test group
            let mut ok_results = 0;
            let mut generated_json_details: Vec<String> = vec![];
            for (test, result) in tgi.tests.iter().zip(tgi.results.iter()) {
                match result {
                    TestResult::TestOk => {
                        ok_results += 1;
                    }
                    TestResult::TestTimeout {
                        captured_stdout,
                        captured_stderr,
                    } => {
                        generated_md_details.push(additional_md_details.len());
                        generated_json_details
                            .push(format!("Test \"{}\" failed due to timeout.", test.name));
                        self.add_md_test_details(
                            additional_md_details,
                            test,
                            &Some(format!(
                                "Failed due to timeout. Timed out after {} seconds.",
                                test.timeout
                            )),
                            &None,
                            &None,
                            &None,
                            &None,
                            &captured_stdout,
                            &captured_stderr,
                            &None,
                        );
                    }
                    TestResult::TestFailed(Some(details)) => {
                        generated_md_details.push(additional_md_details.len());
                        generated_json_details.push(format!("Test \"{}\" failed.", test.name));
                        self.add_md_test_details(
                            additional_md_details,
                            test,
                            &details.message,
                            &details.mismatch_code,
                            &details.mismatch_stdout,
                            &details.mismatch_stderr,
                            &details.captured_code,
                            &details.captured_stdout,
                            &details.captured_stderr,
                            &details.generated_asm,
                        );
                    }
                    TestResult::TestFailed(None) => {}
                    TestResult::TestOutputLimitExceeded { limit } => {
                        generated_md_details.push(additional_md_details.len());
                        generated_json_details.push(format!(
                            "Test \"{}\" failed due to output limit.",
                            test.name
                        ));
                        self.add_md_test_details(
                            additional_md_details,
                            test,
                            &Some(format!("Failed due output exceeding {} bytes.", limit)),
                            &None,
                            &None,
                            &None,
                            &None,
                            &None,
                            &None,
                            &None,
                        );
                    }
                }
            }
            if ok_results != tgi.tests.len() {
                tgi_ok = false;
            }
            if tgi.results.len() < tgi.tests.len() {
                title_suffix = format!(" ({}/{} tests run)", tgi.results.len(), tgi.tests.len());
                tgi_skipped = true;
            } else {
                title_suffix = format!(" ({}/{} tests passed)", ok_results, tgi.tests.len());
            }
            json_details["test_info"] = json::object! {tests_passed: ok_results, total_tests: tgi.tests.len(), test_details: generated_json_details};
        }

        // Fetch results for subgroups
        let mut sub_md_strs: Vec<String> = vec![];
        let mut sub_json: Vec<json::JsonValue> = vec![];
        for sub_tgi in tgi.subgroup_iterators.iter() {
            let (sub_failed, sub_skipped, sub_md_str, sub_json_result) =
                self.recursively_fetch(sub_tgi, indent + 2, additional_md_details);
            tgi_skipped = tgi_skipped || sub_skipped;
            tgi_ok = tgi_ok && sub_failed;
            sub_md_strs.push(sub_md_str);
            sub_json.push(sub_json_result);
        }

        json_details["subgroups"] = sub_json.into();

        let status_symbol = if tgi_skipped {
            self.md_settings.symbol_skipped.as_str()
        } else if tgi_ok {
            self.md_settings.symbol_ok.as_str()
        } else {
            self.md_settings.symbol_failed.as_str()
        };

        let mut md_str = format!(
            "{} * {} {}{}{}{}",
            " ".repeat(indent),
            status_symbol,
            if indent == 0 { "**" } else { "" },
            tgi.title,
            if indent == 0 { "**" } else { "" },
            title_suffix
        );

        if generated_md_details.len() > 0 {
            md_str.push_str(&format!(
                "\n{}   [{}]",
                " ".repeat(indent),
                generated_md_details
                    .iter()
                    .map(|i| format!("<a href=\"#detail-summary-{}\">Detail {}</a>", i + 1, i + 1))
                    .join(", ")
            ));
        }

        // This is extremely inefficient, should use a list of lines instead
        for substr in sub_md_strs.iter() {
            md_str.push_str("\n");
            md_str.push_str(substr);
        }

        (tgi_ok, tgi_skipped, md_str, json_details)
    }

    /// Generate MD test details.
    fn add_md_test_details(
        &self,
        additional_md_details: &mut Vec<String>,
        test: &Test,
        message: &Option<String>,
        mismatch_code: &Option<(i32, i32)>,
        mismatch_stdout: &Option<(String, Vec<String>)>,
        mismatch_stderr: &Option<(String, Vec<String>)>,
        captured_code: &Option<i32>,
        captured_stdout: &Option<String>,
        captured_stderr: &Option<String>,
        generated_asm: &Option<String>,
    ) {
        let mut msg: String = format!("## Additional Details {}", additional_md_details.len() + 1);

        if let Some(extra_msg) = message {
            msg.push_str("\n\n");
            msg.push_str(extra_msg);
        }

        let mut failure_causes: Vec<&str> = vec![];
        if mismatch_code.is_some() {
            failure_causes.push("Incorrect return code.");
        }
        if mismatch_stdout.is_some() {
            failure_causes.push("Incorrect standard output value.");
        }
        if mismatch_stderr.is_some() {
            failure_causes.push("Incorrect standard error value.");
        }
        if failure_causes.len() > 0 {
            msg.push_str("\n\n**Test failed for the following reasons:**\n");
            for cause in failure_causes.iter() {
                msg.push_str("\n* ");
                msg.push_str(cause);
            }
        }

        if let Some(test_desc) = &test.description {
            msg.push_str("\n\n**Test Description:**\n\n");
            msg.push_str(test_desc);
        }

        // Push functions
        fn push_stdin(msg: &mut String, stdin: &str) {
            msg.push_str("\n\n### Standard Input\n```\n");
            msg.push_str(stdin);
            msg.push_str("\n```");
        }
        fn push_inputfile<P: AsRef<Path>>(msg: &mut String, path: P) {
            let path: &Path = path.as_ref();
            msg.push_str("\n\n### Input File\n```");
            if let Some(ex) = path.extension() {
                msg.push_str(ex.to_str().unwrap_or(""));
            }
            msg.push_str("\n");
            let s = std::fs::File::open(path)
                .and_then(|mut f| {
                    let mut buf = String::new();
                    f.read_to_string(&mut buf)?;
                    Ok(buf)
                })
                .unwrap_or("<Unknown file>".to_string());
            msg.push_str(&s);
            msg.push_str("\n```");
        }

        match &test.kind {
            Testkind::Run(kind) => {
                msg.push_str(&format!("\n\n**Command:** `./{}", kind.base.bin));
                for arg in kind.base.args.iter() {
                    msg.push_str(&format!(" {}", arg));
                }
                if kind.input_file.is_some() {
                    msg.push_str(" INPUT_FILE");
                }
                msg.push_str("`");

                if !kind.base.ignore_stdin {
                    push_stdin(&mut msg, &kind.base.stdin);
                }
                if let Some(path) = &kind.input_file {
                    push_inputfile(&mut msg, path);
                }
            }
            Testkind::GenASMAndRun(kind) => {
                msg.push_str(&format!("\n\n**Command:** `./{}", kind.base.bin));
                for arg in kind.base.args.iter() {
                    msg.push_str(&format!(" {}", arg));
                }
                msg.push_str(" INPUT_FILE`");

                if !kind.base.ignore_stdin {
                    push_stdin(&mut msg, &kind.base.stdin);
                }
                push_inputfile(&mut msg, &kind.input_file);
            }
            Testkind::CheckFileExists(kind) => {
                msg.push_str(&format!("\n\n**Expected file:** `{}`", kind.path));

                if !kind.ignore_mimetype {
                    msg.push_str(&format!(
                        "\n\nExpected MIME type: `{}`",
                        kind.mimetype_prefix
                    ));
                }
            }
        }

        if let Some((recv_code, expected_code)) = mismatch_code {
            msg.push_str("\n\n### Code Mismatch\n");
            msg.push_str(&format!(
                "Expected code `{}`. Received code `{}`.",
                expected_code, recv_code
            ));
        }

        if let Some((recv_stdout, expected_stdout)) = mismatch_stdout {
            msg.push_str("\n\n### Standard Output Mismatch\n\n");
            msg.push_str("**Expected stdout:**\n\n");
            msg.push_str(
                &expected_stdout
                    .iter()
                    .map(md_preformatted)
                    .join("\n\nor\n\n"),
            );
            msg.push_str("\n\n**Received stdout:**\n\n");
            msg.push_str(&md_preformatted_with_truncation(
                recv_stdout,
                Some(self.truncate_len),
            ));
        }

        if let Some((recv_stderr, expected_stderr)) = mismatch_stderr {
            msg.push_str("\n\n### Standard Error Mismatch\n\n");
            msg.push_str("**Expected stderr:**\n\n");
            msg.push_str(
                &expected_stderr
                    .iter()
                    .map(md_preformatted)
                    .join("\n\nor\n\n"),
            );
            msg.push_str("\n\n**Received stderr:**\n\n");
            msg.push_str(&md_preformatted_with_truncation(
                recv_stderr,
                Some(self.truncate_len),
            ));
        }

        if let Some(code) = captured_code {
            msg.push_str("\n\n### Captured Code\n");
            msg.push_str(&format!("Output code `{}`.", code));
        }

        if let Some(stdout) = captured_stdout {
            msg.push_str("\n\n### Captured Standard Output\n\n");
            msg.push_str(&md_preformatted_with_truncation(
                stdout,
                Some(self.truncate_len),
            ));
        }

        if let Some(stderr) = captured_stderr {
            msg.push_str("\n\n### Captured Standard Error\n\n");
            msg.push_str(&md_preformatted_with_truncation(
                stderr,
                Some(self.truncate_len),
            ));
        }

        if let Some(asm) = generated_asm {
            msg.push_str("\n\n### Generated Assembly\n\n");
            msg.push_str(&md_preformatted_with_truncation(
                asm,
                Some(self.truncate_len),
            ));
        }

        additional_md_details.push(msg);
    }
}
