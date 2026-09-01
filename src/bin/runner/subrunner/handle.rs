//! Handle for running and grading all tags that are part of a submission.
use std::{collections::BTreeMap, rc::Rc, time::Duration};

use id2202_autograder::{
    config::{Settings, Tests, TestsLoadingOptions},
    db::{
        conn::DatabaseConnection,
        models::{
            JobStatus, Submission, SubmissionJobPlain, SubmissionJobWithReport, SubmissionOrigin,
        },
    },
    error::Error,
    podman::{Mount, PodmanContainer},
    reporting::{Report, ReportMessage},
    utils::path_absolute_join,
};

use crate::{
    shadow::ShadowRepo,
    subrunner::{container::ContainerInfo, tag_runner::TagRunner, verifier},
};

static ERRMSG_INTERNAL_ERROR: &str = "Internal error when starting job. Contact course staff.";

#[derive(Debug)]
pub struct SubmissionRunnerHandle<'a> {
    /// The directory in which this runner will place artifacts. E.g. cloned
    /// git repositories here, input files for test cases, etc. This directory
    /// should be removed by calling the `cleanup` function.
    pub workspace: String,

    /// Program settings, used to open a connection when a tag finishes.
    settings: &'a Settings,

    /// The submission being graded. Its jobs live in the tag runners, one
    /// each, so only what identifies the submission is kept here.
    submission_id: i64,
    origin: SubmissionOrigin,

    /// The shadow repository that each graded tag is archived to.
    shadow: ShadowRepo<'a>,

    // Internal state variables below
    /// The next tag to run
    next_tag_index: usize,

    /// Runners for each of the tags that are being graded. Any deferred tags
    /// will not be part of this vector.
    tag_runners: Vec<TagRunner<'a>>,

    /// Number collected test details
    tests_collected_details: usize,

    /// Maximum number of failed test cases to show. Any additional failed
    /// tests are hidden.
    tests_max_details: usize,

    /// A flag indicating whether we have cleaned up the test procedure or not.
    /// Attempting to run a test case if this is set to true should result in a
    /// fatal error.
    cleaned_up: bool,
}

impl<'a> SubmissionRunnerHandle<'a> {
    /// Creates a new handle, or returns an error message to be shown to the
    /// user. Internal error messages should be presented as log messages only,
    /// using map_err or inspect_err.
    pub fn new(settings: &'a Settings, sub: Submission, runner_id: i32) -> Result<Self, Report> {
        // Convenient for reporting internal errors
        fn internal_error_report() -> Report {
            Report::Message(ReportMessage { msg: ERRMSG_INTERNAL_ERROR.to_string() })
        }

        let tests = Tests::load(&settings.runner.test_config, TestsLoadingOptions::default())
            .map_err(|e| {
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

        // Step 2: Describe the container each tag is built and graded in. Each
        // tag runner gets its own, since a container is removed when dropped.
        let container = || ContainerInfo {
            podman: {
                let mut c = PodmanContainer::new(
                    settings.runner.podman_image.clone(),
                    format!("id2202_runner{}", runner_id),
                );
                c.network = Some(format!("{}{}", settings.runner.podman_network_prefix, runner_id));
                c
            },
            internal_build_dir: "/root/graded_solution".to_string(),
            solution_dir: Mount {
                host_path: solution_dir.clone(),
                container_path: settings.runner.mount_repo.clone(),
                writable: false,
            },
            tests_dir: Mount {
                host_path: tests_dir.clone(),
                container_path: settings.runner.mount_tests.clone(),
                writable: false,
            },
        };

        let verifier = Rc::new(
            verifier::Verifier::start(
                settings,
                &format!("id2202_verifier{}", runner_id),
                &path_absolute_join(&workspace_dir, "verifiers").map_err(|e| {
                    log::error!("Could not join verifier directory path: {e}");
                    internal_error_report()
                })?,
                &tests,
            )
            .map_err(|e| {
                log::error!("Could not start the verifier container: {e}");
                internal_error_report()
            })?,
        );

        // Step 3: Collect the tags to grade
        let timeout_total = Duration::from_secs(tests.default.tag.timeout_total.into());
        let tag_runners: BTreeMap<String, TagRunner> = sub
            .jobs
            .iter()
            .cloned()
            .map(|job| {
                // Tags should have been resolved before being inserted into
                // the database. Anything unknown here is an internal error (or
                // that the test definition was updated between being submitted
                // and being picked up by a runner).
                let tag = tests.tags.get(&job.tag).ok_or_else(|| {
                    log::error!("Received unknown grading tag {}", job.tag);
                    internal_error_report()
                })?;
                let tag_name = job.tag.to_owned();
                let runner = TagRunner::new(
                    settings,
                    tag,
                    job,
                    container(),
                    &source_dir,
                    verifier.clone(),
                    timeout_total,
                );
                Ok((tag_name, runner))
            })
            .collect::<Result<_, Report>>()?;

        // Create the source dir and fetch the submission into it
        std::fs::create_dir_all(&source_dir)
            .map_err(Error::from)
            .and_then(|_| sub.origin.fetch_into(settings, &source_dir))
            .map_err(|e| {
                log::error!("Could not fetch the submitted solution from the origin: {e}");
                internal_error_report()
            })?;

        let shadow = ShadowRepo::open(settings, &sub, &workspace_dir).map_err(|e| {
            log::error!("Could not open the shadow repository: {e}");
            internal_error_report()
        })?;

        // Defuse the guard, ensuring that the workspace remains
        scopeguard::ScopeGuard::into_inner(workspace_guard);

        Ok(SubmissionRunnerHandle {
            workspace: workspace_dir,
            settings,
            submission_id: sub.id,
            origin: sub.origin,
            shadow,
            next_tag_index: 0,
            tag_runners: tag_runners.into_values().collect(),
            tests_collected_details: 0,
            tests_max_details: settings.reporting.shown_failures,
            cleaned_up: false,
        })
    }

    /// Returns `true` if every job of this batch has reached a terminal
    /// status. In which case, the results can be collected.
    pub fn is_finished(&self) -> bool {
        self.tag_runners.iter().all(|tr| tr.status.is_finished())
    }

    /// Fails every tag that has not finished, for when the runner itself is
    /// unhealthy rather than the submission. Tags that already reached a
    /// terminal status keep the result they earned.
    pub fn set_as_erroneous(&mut self) -> Result<(), Error> {
        let mut conn = DatabaseConnection::connect(self.settings)?;
        let report = Report::Message(ReportMessage {
            msg: "An internal error occurred while grading your solution. Contact course staff."
                .to_string(),
        });

        // From the tag being graded onwards. Everything before it has written
        // its result already, and the current one is included even when it
        // reached a status of its own, since the write that would have
        // recorded it is what failed.
        let from = self.next_tag_index;
        self.next_tag_index = self.tag_runners.len();

        let pending = &mut self.tag_runners[from..];
        for tr in pending.iter_mut() {
            tr.status = JobStatus::AutograderFailure;
        }

        let (mut started, mut unstarted): (Vec<_>, Vec<_>) =
            pending.iter_mut().partition(|tr| tr.job.started_at.is_some());

        SubmissionJobPlain::set_all_as_voided(
            unstarted.iter_mut().map(|tr| &mut tr.job),
            &mut conn,
            JobStatus::AutograderFailure,
            Some(&report),
        )?;

        SubmissionJobPlain::set_all_as_finished(
            started.iter_mut().map(|tr| &mut tr.job),
            &mut conn,
            JobStatus::AutograderFailure,
            Some(&report),
        )
    }

    /// The submission being graded.
    pub fn submission_id(&self) -> i64 {
        self.submission_id
    }

    /// Where the submission came from, for reporting back to it.
    pub fn origin(&self) -> &SubmissionOrigin {
        &self.origin
    }

    /// The job within the handle together with the report each tag_runner
    /// produced (if it has generated one).
    pub fn job_results(&self) -> Vec<SubmissionJobWithReport> {
        self.tag_runners
            .iter()
            .map(|tr| SubmissionJobWithReport {
                job: tr.job.clone(),
                report: tr.get_report().cloned().map(Report::tag_grading),
            })
            .collect()
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

        let tag_runner = self.tag_runners.get_mut(self.next_tag_index).ok_or_else(|| {
            Error::runtime(format!("expected a tag runner for index {}", self.next_tag_index))
        })?;

        if !tag_runner.has_built() {
            log::debug!("Building project for tag \"{}\"", tag_runner.job.tag);

            // A job starts when its build does. The claim only reserved it, and
            // the jobs of one claim are graded one after another.
            tag_runner.job.set_as_started(&mut DatabaseConnection::connect(self.settings)?)?;

            if !tag_runner.build()? {
                log::info!(
                    "Build failed for tag \"{}\", proceeding to next tag",
                    tag_runner.job.tag
                );
                tag_runner.generate_report()?;
            }
        } else {
            let prev_count = tag_runner.collected_reports;

            if !tag_runner.run_test(self.tests_collected_details < self.tests_max_details)? {
                log::info!(
                    "Finished running test cases for tag \"{}\", proceeding to next tag",
                    tag_runner.job.tag
                );
                tag_runner.generate_report()?;
            }

            if tag_runner.collected_reports > prev_count {
                self.tests_collected_details += 1;
            }
        }

        if tag_runner.status.is_finished() {
            tag_runner.record_to_shadow(&self.shadow)?;

            let report = Report::tag_grading(
                tag_runner
                    .get_report()
                    .ok_or_else(|| {
                        Error::runtime(format!(
                            "expected a report for finished tag \"{}\"",
                            tag_runner.job.tag
                        ))
                    })?
                    .clone(),
            );
            let status = tag_runner.status;

            let mut conn = DatabaseConnection::connect(self.settings)?;
            tag_runner.job.set_as_finished(&mut conn, status, Some(&report))?;

            self.next_tag_index += 1;
        }

        Ok(())
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

impl Drop for SubmissionRunnerHandle<'_> {
    fn drop(&mut self) {
        self.cleanup();
    }
}
