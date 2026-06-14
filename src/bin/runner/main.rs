use clap::Parser;
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
use std::sync::mpsc;
use std::{
    io::Write,
    time::{Duration, Instant},
};

use id2202_autograder::{
    config::Settings,
    db::{
        conn::DatabaseConnection,
        models::{Submission, SubmissionInfo, SubmissionStatusCode},
        notify::listen as db_listen,
    },
    error::Error,
    reporting::{Report, ReportMessage, ReportWrapper},
    utils::{
        create_dir_if_not_exists, path_absolute_join, path_absolute_parent, syscommand_timeout,
        systemtime_to_fsfriendly_utc_string, SyscommandSettings,
    },
};

mod subrunner;
use subrunner::SubmissionRunnerHandle;

use crate::subrunner::tag_runner::TagRunner;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the TOML file containing the program settings
    #[arg(short, long)]
    settings: String,

    /// Runner index. Used for debugging purposes.
    #[arg(short = 'i', long = "index", long = "runner-id")]
    runner_id: i32,
}

const MSG_NOTIFY: &'static str = "notify";
const MSG_SIGNAL: &'static str = "signal";

fn main() -> Result<(), Error> {
    let args: Args = Args::parse();
    let settings = Settings::load(&args.settings)?;

    let logname = format!("runner{}", args.runner_id);
    settings.setup_logging(&logname)?;

    // Check if this runner have any active jobs from a previous process that
    // it needs to handle first... For simplicity, we should just cancel them
    // and notify the user.
    match DatabaseConnection::connect(&settings) {
        Ok(mut conn) => {
            let err_report = Report::Message(ReportMessage {
                msg: format!(
                    "{} {} {}",
                    "The runner was interrupted before it could finish grading your solution.",
                    "Please try to submit your solution again.",
                    "Contact course staff if the problem persists."
                ),
            });
            let mut still_searching = true;
            while still_searching {
                use diesel::{self, ExpressionMethods, QueryDsl, RunQueryDsl, SelectableHelper};
                use id2202_autograder::db::schema::submissions::{
                    self, assigned_runner_id, exec_finished,
                };
                let ret: Result<Submission, _> = submissions::table
                    .select(Submission::as_select())
                    .filter(assigned_runner_id.eq(args.runner_id))
                    .filter(exec_finished.eq(false))
                    .first(&mut conn.conn);
                match ret {
                    Ok(sub) => {
                        log::warn!("Found unfinished submission: {sub:?}");
                        match conn.get_submission_info(sub.id) {
                            Ok(info) => {
                                conn.report_and_status(
                                    &settings,
                                    &info,
                                    &err_report,
                                    SubmissionStatusCode::AutograderFailure,
                                    true,
                                )?;
                            }
                            Err(e) => {
                                log::warn!(
                                    "No source found for unfinished submission {}: {}",
                                    sub.id,
                                    e
                                );
                                conn.set_status(
                                    &sub,
                                    SubmissionStatusCode::AutograderFailure,
                                    true,
                                )?;
                            }
                        }
                    }
                    Err(e) => {
                        log::info!(
                            "No previous submissions found in database ({e}). Proceeding to start runner."
                        );
                        still_searching = false;
                    }
                }
            }
        }
        Err(e) => {
            log::error!("Fatal: Could not connect to database: {e}");
            return Err(e);
        }
    }

    // Message channels from threads -> main thread
    let (msg_send, msg_recv) = std::sync::mpsc::channel();

    // Functionality for interrupting on received signals
    let mut signals = Signals::new(&[SIGINT, SIGTERM])?;
    let sigc_send = msg_send.clone();
    let sigc_handle = std::thread::spawn(move || {
        for sig in signals.forever() {
            log::info!("Received signal {sig}");
            sigc_send
                .send(MSG_SIGNAL)
                .unwrap_or_else(|e| log::error!("Could not send notification message: {e}"));
            break;
        }
    });

    // A notifier thread, checking if the notification file has been modified.
    let (notify_send, notify_recv) = std::sync::mpsc::channel();
    let notify_settings = settings.clone();
    let notify_handle = std::thread::spawn(move || {
        log::debug!("Listener thread spawned");
        let mut watching = true;
        while watching {
            match db_listen(&notify_settings, "submission") {
                Ok(true) => {
                    // Received new event
                    msg_send.send(MSG_NOTIFY).unwrap_or_else({
                        |e| {
                            log::error!("Could not send notification message: {e:#}");
                            watching = false;
                        }
                    });
                }
                Ok(false) => {} // timed out
                Err(e) => {
                    log::error!("Received error while listening on new submissions: {e:#}");
                    watching = false;
                }
            }

            // Check if the main thread want's us to shut down
            if let Ok(msg) = notify_recv.try_recv() {
                log::info!(
                    "Received notify message \"{msg}\" from main thread, stopping inotify thread."
                );
                watching = false;
            }
        }
        log::debug!("Listener thread finished");
        msg_send
            .send(MSG_NOTIFY)
            .unwrap_or_else(|e| log::error!("Could not send notification message: {e}"));
    });

    // Polling frequency for new jobs
    let init_time = Instant::now();
    let interval = Duration::from_secs(settings.runner.database_poll_interval_seconds.into());
    let mut next_offset = Duration::ZERO;

    // Handle for managing active jobs
    // (Use .take() to set this to None)
    let mut active_sub: Option<SubmissionRunnerHandle> = None;

    // Some important notes on this "main loop":
    //
    // It is possible that the subrunner may throw an error, and that must be
    // reported back to the submission source. Any failure on reporting back to
    // the user is considered fatal however, and those failures should
    // terminate the runner. In this case the runner should be restarted by the
    // entrypoint, and the first thing the runner will do is to set the status
    // of any unfinished submissions.
    let mut active = true;
    while active {
        if let Some(run_handle) = active_sub.as_mut() {
            // Run a step in the action submission. This is a non-blocking
            // operation that is run in a small-step semantics manner by
            // repeatedly calling `run_next`. We have a separate message
            // check in this branch since we do not want it to block if we
            // are running a job.

            if let Err(e) = run_handle.run_next() {
                // Runtime error when running submission.
                // This is not the same as when failing a test case or when
                // there is a build or timeout error.
                log::error!("Received error when running a job: {e}");
                run_handle.set_as_erroneous();
            }

            if run_handle.is_finished() {
                let mut conn = DatabaseConnection::connect(&settings)?;

                let subinfo = conn.get_submission_info(run_handle.submission_id)?;

                // 1. First record the graded files and tag
                //    results on the shadow repository.
                let (report, status) = match record_to_shadow(
                    &settings,
                    &subinfo,
                    &run_handle.workspace,
                    &run_handle.source_dir,
                    &run_handle.get_tag_runners(),
                ) {
                    Ok(()) => (run_handle.compile_report(), run_handle.get_status_code()),
                    Err(e) => {
                        // Not being able to record info in the
                        // shadow repository is a fatal error,
                        // as that is being used to check
                        // whether a student has passed or not.
                        log::error!("Could not commit results to shadow repository: {e}");
                        (
                            Report::Message(ReportMessage {
                                msg: format!(
                                    "Error grading submission {}. Contact course staff.",
                                    subinfo.get_submission().id,
                                ),
                            }),
                            SubmissionStatusCode::AutograderFailure,
                        )
                    }
                };

                // 2. Write a comment to the GitHub commit
                conn.report_and_status(&settings, &subinfo, &report, status, true)
                    .unwrap_or_else(|e| {
                        log::warn!("Could not set commit message and/or status: {e}")
                    });
                conn.set_exec_date_finished(subinfo.get_submission().id)
                    .unwrap_or_else(|e| {
                        log::warn!(
                            "Could not set finish date for job {}: {}",
                            subinfo.get_submission().id,
                            e
                        )
                    });

                log::info!("Grading of submission {} done.", run_handle.submission_id);
                run_handle.cleanup();
                active_sub.take();
            }

            match msg_recv.try_recv() {
                Ok(cause) => {
                    if cause == MSG_SIGNAL {
                        log::warn!("Received shutdown signal during an active job, cancelling the running job.");
                        if let Some(run_handle) = active_sub.as_mut() {
                            run_handle.cleanup();
                        }
                        active_sub.take();
                        active = false;
                    } else if cause == MSG_NOTIFY {
                        log::debug!("Received notification, but ignoring this since we are already running a job.");
                    } else {
                        log::error!("Received invalid notification cause \"{cause}\". Cancelling current job");
                        if let Some(run_handle) = active_sub.as_mut() {
                            run_handle.cleanup();
                        }
                        active_sub.take();
                        active = false;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // no notification received, completely normal.
                }
                Err(e) => {
                    log::warn!("Received unexpected channel error: {e}")
                }
            }
            next_offset = init_time.elapsed();
        } else {
            // Check if there are new jobs to be run in the database. This will
            // be done periodically on the specified interval, or when we
            // receive a notification.
            next_offset += interval;
            log::debug!("Checking if any new jobs are available");

            let mut conn = DatabaseConnection::connect(&settings)?;

            match conn.try_assign_submission(args.runner_id)? {
                Some(sub) => {
                    log::info!("Assigned submission: {:#?}", sub);

                    let subinfo = conn.get_submission_info(sub.id)?;

                    match SubmissionRunnerHandle::new(&settings, &subinfo, args.runner_id) {
                        Ok(handle) => {
                            active_sub = Some(handle);
                            conn.report_and_status(
                                    &settings,
                                    &subinfo,
                                    &Report::Message(ReportMessage { msg: format!("{} {}",
                                        "The autograder is now running your submission.",
                                        "The results will be provided as a comment here when they are ready."
                                    )}),
                                    SubmissionStatusCode::Running,
                                    false,
                                )
                                .unwrap_or_else(|e| {
                                    log::warn!("Could not set commit message and/or status: {e}")
                                });
                            conn.set_exec_date_started(sub.id).unwrap_or_else(|e| {
                                log::warn!("Could not set start date for job {}: {}", sub.id, e)
                            });
                            // Do not wait for a timeout, just proceed
                            // to run the test cases.
                            next_offset = init_time.elapsed();
                        }
                        Err(report) => {
                            conn.report_and_status(
                                &settings,
                                &subinfo,
                                &Report::Wrapper(ReportWrapper {
                                    title: Some("Your submission could not be graded.".to_string()),
                                    reports: vec![report],
                                }),
                                SubmissionStatusCode::SubmissionError,
                                true,
                            )
                            .unwrap_or_else(|e| {
                                log::warn!("Could not set commit message and/or status: {e}")
                            });

                            conn.set_exec_date_finished(sub.id)
                                .unwrap_or_else(|e| log::warn!("Could not set exec finished: {e}"));
                        }
                    }
                }
                // No new job
                None => {}
            }

            let sleep_time = next_offset
                .checked_sub(init_time.elapsed())
                .unwrap_or(Duration::ZERO);
            match msg_recv.recv_timeout(sleep_time) {
                Ok(cause) => {
                    if cause == MSG_SIGNAL {
                        // Received a message on the signal channel, no longer running
                        active = false;
                    } else if cause == MSG_NOTIFY {
                        log::debug!("Received notification, TODO: check the database for new jobs");
                    } else {
                        log::error!("Received invalid notification cause \"{cause}\"");
                        active = false;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // timeout expected
                }
                Err(e) => {
                    log::warn!("Received unexpected channel error: {e}")
                }
            }
        }
        if active && sigc_handle.is_finished() {
            log::error!("Signal handler finished unexpectedly.");
            active = false;
        }
        if notify_handle.is_finished() {
            log::error!("Notification handler finished unexpectedly. Sending termination signal");
        }
    }

    // Cancel the notification thread if still active
    if !notify_handle.is_finished() {
        notify_send
            .send(MSG_NOTIFY)
            .unwrap_or_else(|e| log::warn!("Could not tell inotify thread to exit: {e}"))
    }

    // TODO: The notify_handle will be dropped here. Should try to have a more
    // smooth join of threads. This is not a big deal, but would be nice.

    log::info!("Runner {} exiting", args.runner_id);
    Ok(())
}

/// Commits the repository files in `repo_dir` and creates new ones from the
/// `files` list to the shadow repository for this submission.
fn record_to_shadow(
    settings: &Settings,
    subinfo: &SubmissionInfo,
    workspace_dir: &str,
    source_dir: &str,
    tag_runners: &[TagRunner],
) -> Result<(), Error> {
    // 1. Create the shadow repository if it does not exist.
    let shadow_repo = match subinfo {
        SubmissionInfo::GitHub {
            sub: _,
            src: _,
            gh_src,
            gh_info: _,
        } => path_absolute_join(
            &settings.runner.shadow_dir,
            format!(
                "github/{}/{}/{}.git",
                gh_src.domain, gh_src.org, gh_src.repo
            ),
        )?,
        SubmissionInfo::GitLab {
            sub: _,
            src: _,
            gl_src,
            gl_info: _,
        } => path_absolute_join(
            &settings.runner.shadow_dir,
            format!(
                "gitlab/{}/{}/{}.git",
                gl_src.domain, gl_src.namespace, gl_src.repo
            ),
        )?,
    };
    if !std::fs::exists(&shadow_repo)? {
        log::info!("The shadow repository does not exist. Creating new shadow repository at path {shadow_repo}");
        std::fs::create_dir_all(&shadow_repo)?;

        syscommand_timeout(
            ["git", "-C", &shadow_repo, "init", "--bare"],
            SyscommandSettings {
                expected_code: Some(0),
                ..Default::default()
            },
        )?;
    }

    // 2. Clone the shadow repository.

    // Ensure that the workspace dir exists
    create_dir_if_not_exists(workspace_dir)?;

    // Set up necessary paths
    let shadow_dir = path_absolute_join(workspace_dir, "shadow")?;

    let date_dir = path_absolute_join(
        &shadow_dir,
        systemtime_to_fsfriendly_utc_string(&subinfo.get_submission().date_submitted)
            .ok_or_else(|| Error::convert("could not create date for date dir"))?,
    )?;
    let snapshot_dir = path_absolute_join(&shadow_dir, "snapshot")?;

    log::debug!("Cloning shadow directory {shadow_repo} to {shadow_dir}");
    syscommand_timeout(
        &["git", "clone", "--local", &shadow_repo, &shadow_dir],
        SyscommandSettings {
            expected_code: Some(0),
            ..Default::default()
        },
    )
    .inspect_err(|e| log::error!("Could not clone shadow repo {shadow_repo}: {e}"))?;

    syscommand_timeout(
        &[
            "git",
            "-C",
            &shadow_dir,
            "config",
            "--local",
            "user.name",
            &settings.name,
        ],
        SyscommandSettings {
            expected_code: Some(0),
            ..Default::default()
        },
    )
    .inspect_err(|e| log::error!("Could not set git config for shadow repo: {e}"))?;

    syscommand_timeout(
        &[
            "git",
            "-C",
            &shadow_dir,
            "config",
            "--local",
            "user.email",
            "id2202@localhost",
        ],
        SyscommandSettings {
            expected_code: Some(0),
            ..Default::default()
        },
    )
    .inspect_err(|e| log::error!("Could not set git config for shadow repo: {e}"))?;

    // 3. Add the new files and push

    std::fs::create_dir(&date_dir)?;

    for tr in tag_runners {
        // Create the report
        let report = tr.results_report();
        let content_path = path_absolute_join(&date_dir, format!("{}.results.json", &tr.tag_name))?;
        let mut f = std::fs::File::create(content_path)?;
        f.write(report.to_json()?.as_bytes())?;

        // Create the snapshot for this solution only if it attempted to
        // actually build the project. If this is not the case, then there is
        // something wrong with the tag source directory and these files may
        // contain bad files that should not be stored.
        if tr.attempted_build() {
            let graded_src_dir = path_absolute_join(&source_dir, &tr.build_conf.srcdir)?;
            let target_src_dir = path_absolute_join(&snapshot_dir, &tr.build_conf.srcdir)?;
            let target_src_parent = path_absolute_parent(&target_src_dir)?;

            if !std::fs::exists(&target_src_parent)? {
                std::fs::create_dir_all(&target_src_parent)?;
            }
            // Remove the previous solution if exists
            if std::fs::exists(&target_src_dir)? {
                std::fs::remove_dir_all(&target_src_dir)?;
            }

            dircpy::copy_dir(&graded_src_dir, &target_src_dir)?;
        }
    }

    let mut cmdadd: Vec<&str> = vec!["git", "-C", &shadow_dir, "add", &date_dir];
    if std::fs::exists(&snapshot_dir)? {
        cmdadd.push(&snapshot_dir);
    }

    syscommand_timeout(
        cmdadd.as_slice(),
        SyscommandSettings {
            expected_code: Some(0),
            ..Default::default()
        },
    )
    .inspect_err(|e| log::error!("Could not add files to shadow repo {shadow_repo}: {e}"))?;

    let commit_msg = format!("Results for submission {}", subinfo.get_submission().id);
    syscommand_timeout(
        &[
            "git",
            "-C",
            &shadow_dir,
            "commit",
            "--allow-empty",
            "-m",
            &commit_msg,
        ],
        SyscommandSettings {
            expected_code: Some(0),
            ..Default::default()
        },
    )
    .inspect_err(|e| log::error!("Could not commit files to shadow repo {shadow_repo}: {e}"))?;

    syscommand_timeout(
        &["git", "-C", &shadow_dir, "push"],
        SyscommandSettings {
            expected_code: Some(0),
            ..Default::default()
        },
    )
    .inspect_err(|e| log::error!("Could not push files to shadow repo {shadow_repo}: {e}"))?;

    Ok(())
}
