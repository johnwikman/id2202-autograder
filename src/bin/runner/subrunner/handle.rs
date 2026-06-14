/// Handle for running and grading all tags that are part of a submission.
use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime},
};

use id2202_autograder::{
    config::{Settings, Tests},
    db::models::{SubmissionInfo, SubmissionStatusCode},
    error::Error,
    reporting::{Report, ReportInvalidTag, ReportMessage, ReportSubmission, ReportTagGrading},
    utils::{path_absolute_join, syscommand_timeout, SyscommandSettings},
};

use crate::subrunner::{container::ContainerInfo, tag_runner::TagRunner};

static ERRMSG_INTERNAL_ERROR: &str = "Internal error when starting job. Contact course staff.";

#[derive(Debug, Clone)]
pub struct SubmissionRunnerHandle {
    /// The directory in which this runner will place artifacts. E.g. cloned
    /// git repositories here, input files for test cases, etc. This directory
    /// should be removed by calling the `cleanup` function.
    pub workspace: String,

    /// ID of the current submission.
    pub submission_id: i64,

    /// The path to the repository containing the submitted source code that
    /// should be graded. The runner will not modify this directory, and will
    /// instead copy the parts necessary to grade a specfific tag before
    /// proceeding to build the project.
    pub source_dir: String,

    // Internal state variables below
    /// The next tag to run
    next_tag_index: usize,

    /// Test configuration and iterators over tags and their test groups. Using
    /// this for storing test information and progress together.
    tag_runners: Vec<TagRunner>,

    /// Number collected test details
    tests_collected_details: usize,

    /// Maximum number of failed test cases to show. Any additional failed
    /// tests are hidden.
    tests_max_details: usize,

    /// Deadline time. If we have reached or exceeded this timestamp, then we
    /// the total grading procedure has timed out.
    deadline_time: SystemTime,

    /// A flag indicating whether we have cleaned up the test procedure or not.
    /// Attempting to run a test case if this is set to true should result in a
    /// fatal error.
    cleaned_up: bool,

    /// Causes of overall test failures that should be reported back to the
    /// student.
    status_code: SubmissionStatusCode,
}

impl SubmissionRunnerHandle {
    /// Creates a new handle, or returns an error message to be shown to the
    /// user. Internal error messages should be presented as log messages only,
    /// using map_err or inspect_err.
    pub fn new(
        settings: &Settings,
        subinfo: &SubmissionInfo,
        runner_id: i32,
    ) -> Result<Self, Report> {
        // Convenient for reporting internal errors
        fn internal_error_report() -> Report {
            Report::Message(ReportMessage {
                msg: ERRMSG_INTERNAL_ERROR.to_string(),
            })
        }

        let sub = subinfo.get_submission();

        let tests = Tests::load(&settings.runner.test_config).map_err(|e| {
            log::error!("Could not load test configuration: {e}");
            internal_error_report()
        })?;

        // Step 1: Set up the workspace and information about the directories within.
        let workspace_dir = path_absolute_join(
            &settings.runner.workspace_dir,
            format!("runner{runner_id}_{:08x}", rand::random::<u32>()),
        )
        .map_err(|e| {
            log::error!("Could not join workspace_dir path: {e}");
            internal_error_report()
        })?;
        match std::fs::exists(&workspace_dir) {
            // Expected case
            Ok(false) => {}
            Ok(true) => {
                log::error!("Collision in workspace_dir: {workspace_dir}");
                return Err(internal_error_report());
            }
            Err(e) => {
                log::error!("Could not check for existence in filesystem: {e}");
                return Err(internal_error_report());
            }
        }
        std::fs::create_dir_all(&workspace_dir).map_err(|e| {
            log::error!("Could not create workspace_dir: {e}");
            internal_error_report()
        })?;

        // Set up a guard for the workspace, such that it gets removed in case
        // we perform an early exit from this function.
        let workspace_guard = scopeguard::guard(&workspace_dir, |path| {
            std::fs::remove_dir_all(path)
                .unwrap_or_else(|e| log::error!("Could not clean up workspace_dir: {e}"));
        });

        let source_dir = path_absolute_join(&workspace_dir, "source").map_err(|e| {
            log::error!("Could not join source_dir path: {e}");
            internal_error_report()
        })?;

        let solution_dir = path_absolute_join(&workspace_dir, "solution").map_err(|e| {
            log::error!("Could not join solution_dir path: {e}");
            internal_error_report()
        })?;

        let tests_dir = path_absolute_join(&workspace_dir, "tests").map_err(|e| {
            log::error!("Could not join tests_dir path: {e}");
            internal_error_report()
        })?;

        // Step 2: Set up the container
        let container = ContainerInfo {
            podman_image: settings.runner.podman_image.clone(),
            podman_container_name: format!("id2202_runner{}", runner_id),
            podman_network_name: format!("{}{}", settings.runner.podman_network_prefix, runner_id),
            internal_build_dir: "/root/graded_solution".to_string(),
            mount_solution: settings.runner.mount_repo.clone(),
            mount_tests: settings.runner.mount_tests.clone(),
            external_solution: solution_dir,
            external_tests: tests_dir,
        };

        // Step 3: Collect the tags to grade
        log::debug!(
            "Collecting grading tag information from {}",
            sub.grading_tags
        );
        let mut tag_runners: BTreeMap<String, TagRunner> = BTreeMap::new();
        for t in sub.grading_tags.split(";") {
            match tests.tag_groups.get(t) {
                Some(vec_tag) => {
                    for tag in vec_tag {
                        match tag_runners.get_mut(&tag.name) {
                            Some(runner) => {
                                runner.derived_from.insert(t.to_owned());
                            }
                            None => {
                                let mut runner =
                                    TagRunner::new(tag, &container, &tests.default, &source_dir);
                                runner.derived_from.insert(t.to_owned());
                                tag_runners.insert(tag.name.clone(), runner);
                            }
                        }
                    }
                }
                None => {
                    log::info!("Received invalid tag {t}");
                    // Format the error message on a GitHub Markdown friendly format
                    let mut direct_tags: Vec<String> = vec![];
                    let mut tag_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
                    for (k, tags) in tests.tag_groups.iter() {
                        if tags.len() == 1 && tags.get(0).map_or(false, |tag| tag.name == *k) {
                            direct_tags.push(k.to_owned());
                        } else {
                            tag_groups.insert(
                                k.to_owned(),
                                tags.iter().map(|tag| tag.name.to_owned()).collect(),
                            );
                        }
                    }

                    return Err(Report::InvalidTag(ReportInvalidTag {
                        tag_name: t.to_string(),
                        known_grading_tags: direct_tags,
                        known_tag_groups: tag_groups,
                    }));
                }
            }
        }

        // After this point we need to make sure that the workspace_dir is
        // deleted when the TestRunnerHandle is dropped.

        let (ssh_url, commit) = subinfo.ssh_url_and_commit();

        // A way to check out a specific commit, without the cloning the whole history
        let gitcmd_settings = SyscommandSettings {
            expected_code: Some(0),
            ..Default::default()
        };
        std::fs::create_dir_all(&source_dir)
            .map_err(Error::from)
            .and_then(|_| {
                syscommand_timeout(
                    &["git", "-C", &source_dir, "init"],
                    gitcmd_settings.to_owned(),
                )
            })
            .and_then(|_| {
                syscommand_timeout(
                    &["git", "-C", &source_dir, "remote", "add", "origin", ssh_url],
                    gitcmd_settings.to_owned(),
                )
            })
            .and_then(|_| {
                syscommand_timeout(
                    &[
                        "git",
                        "-C",
                        &source_dir,
                        "fetch",
                        "--depth",
                        "1",
                        "origin",
                        commit,
                    ],
                    gitcmd_settings.to_owned(),
                )
            })
            .and_then(|_| {
                syscommand_timeout(
                    &["git", "-C", &source_dir, "checkout", "FETCH_HEAD"],
                    gitcmd_settings.to_owned(),
                )
            })
            .map_err(|e| {
                log::error!("Error cloning repository from {}: {e}", ssh_url);
                internal_error_report()
            })?;

        let deadline_time = SystemTime::now()
            .checked_add(Duration::from_secs(tests.default.timeout_total.into()))
            .ok_or_else(|| {
                log::error!("Internal error setting deadline date.");
                internal_error_report()
            })?;

        // Defuse the guard, ensuring that the workspace remains
        scopeguard::ScopeGuard::into_inner(workspace_guard);

        Ok(SubmissionRunnerHandle {
            workspace: workspace_dir,
            submission_id: sub.id,
            source_dir: source_dir,
            next_tag_index: 0,
            tag_runners: tag_runners.into_values().collect(),
            tests_collected_details: 0,
            tests_max_details: tests.default.shown_failures,
            deadline_time: deadline_time,
            cleaned_up: false,
            status_code: SubmissionStatusCode::Running,
        })
    }

    /// Returns `true` if handle has finished running all the test cases. In
    /// which case, the results can be collected.
    pub fn is_finished(&self) -> bool {
        self.status_code.is_finished() || self.next_tag_index >= self.tag_runners.len()
    }

    /// Sets the handle as erroneous from an external perspective, making sure
    /// that we will not proceed to run any other test cases. If the existing
    /// status code is an error, that will be preserved.
    pub fn set_as_erroneous(&mut self) {
        if !self.status_code.is_error() {
            self.status_code = SubmissionStatusCode::AutograderFailure;
        }
    }

    /// Returns the status code of the handle.
    pub fn get_status_code(&self) -> SubmissionStatusCode {
        self.status_code
    }

    /// Returns a slice over the tag runners contained within this handle.
    /// Useful for read-only access to the data contained within.
    pub fn get_tag_runners(&self) -> &[TagRunner] {
        &self.tag_runners
    }

    /// Run the next part of the testing runner process.
    ///
    /// This is the main aspect of the SubmissionRunnerHandle. The owner of this
    /// handle should call this function in a loop until `is_finished()`
    /// returns `true`. This can be seen as a form of small-step semantics.
    ///
    /// If this function returns an error, that is something that has gone
    /// wrong with the runner itself, not with a test case. If a test case
    /// could be successfully run (even if it timed out or otherwise failed),
    /// then this will return an Ok with a unit value. An Err return value
    /// indicates that something has gone wrong with the grading process itself
    /// and that the TestRunnerHandle must stop.
    pub fn run_next(&mut self) -> Result<(), Error> {
        if self.is_finished() {
            log::debug!(
                "next_tag_index: {}, tag_runners.len(): {}",
                self.next_tag_index,
                self.tag_runners.len()
            );
            return Error::err_runtime(
                "Attempted to run the next test case after the submission handle had finished.",
            );
        }

        if SystemTime::now() >= self.deadline_time {
            log::info!("Grading process timed out globally");
            self.status_code = SubmissionStatusCode::SubmissionTimedOut;
        }

        let tag_runner = self
            .tag_runners
            .get_mut(self.next_tag_index)
            .ok_or_else(|| {
                Error::runtime(format!(
                    "expected a tag runner for index {}",
                    self.next_tag_index
                ))
            })?;

        if !tag_runner.has_built() {
            log::debug!("Building project for tag \"{}\"", tag_runner.tag_name);
            if !tag_runner.build()? {
                log::info!(
                    "Build failed for tag \"{}\", proceeding to next tag",
                    tag_runner.tag_name
                );
                self.next_tag_index += 1;
            }
        } else {
            let prev_count = tag_runner.collected_reports;

            if !tag_runner.run_test(self.tests_collected_details < self.tests_max_details)? {
                log::info!(
                    "Finished running test cases for tag \"{}\", proceeding to next tag",
                    tag_runner.tag_name
                );
                self.next_tag_index += 1;
            }

            if tag_runner.collected_reports > prev_count {
                self.tests_collected_details += 1;
            }
        }
        if let Some(ssc) = tag_runner.experienced_bad_behavior() {
            self.status_code = ssc;
        }

        Ok(())
    }

    /// Compiles the report of submission results, and sets the status of the
    /// submission if it is still considered as running.
    ///
    pub fn compile_report(&mut self) -> Report {
        let tag_reports: Vec<ReportTagGrading> = self
            .tag_runners
            .iter()
            .map(|tr| tr.results_report())
            .collect();
        if !self.status_code.is_finished() {
            if tag_reports.iter().all(|tr| tr.ok) {
                self.status_code = SubmissionStatusCode::Success;
            } else if tag_reports.iter().any(|tr| tr.build_failure.is_some()) {
                self.status_code = SubmissionStatusCode::BuildError;
            } else {
                self.status_code = SubmissionStatusCode::TestCasesFailed;
            }
        }

        Report::Submission(ReportSubmission {
            premature_exit_reason: match self.status_code {
                SubmissionStatusCode::AutograderFailure | SubmissionStatusCode::NotStarted => {
                    Some("Grading process was interrupted. Contact course staff.".to_string())
                }
                SubmissionStatusCode::SubmissionError => {
                    Some("There was an error with the submission.".to_string())
                }
                SubmissionStatusCode::SubmissionTimedOut => {
                    Some("The submission timed out.".to_string())
                }
                _ => None,
            },
            max_shown_details: Some(self.tests_max_details),
            tag_reports: tag_reports,
        })
    }

    /// Performs a cleanup, removing any lingering files
    pub fn cleanup(&mut self) {
        log::debug!("Cleaning up each of the tag runners");
        for tr in self.tag_runners.iter_mut() {
            if let Err(e) = tr.cleanup() {
                log::warn!("Could not perform cleanup on one of the tag runners: {e}");
            }
        }

        if std::fs::exists(&self.workspace).unwrap_or_else(|e| {log::warn!("Could not check existence of workspace directory: {e}\n\nAssuming it does not exist"); false}) {
            log::debug!(
                "Removing the workspace directory \"{}\"",
                self.workspace
            );
            if let Err(e) = std::fs::remove_dir_all(&self.workspace) {
                log::warn!("Could not remove the workspace directory: {e}");
            }
        }

        self.cleaned_up = true;
    }
}

impl Drop for SubmissionRunnerHandle {
    fn drop(&mut self) {
        self.cleanup();
    }
}
