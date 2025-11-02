use clap::Parser;
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
use std::sync::mpsc;
use std::{
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};

use id2202_autograder::{
    db::{
        conn::DatabaseConnection,
        models::{Submission, SubmissionStatusCode},
    },
    error::Error,
    github,
    notify::Listener,
    settings::Settings,
    utils::{
        create_dir_if_not_exists, path_absolute_join, syscommand_timeout,
        systemtime_to_fsfriendly_utc_string, SyscommandSettings,
    },
};

mod testrunner;
use testrunner::TestRunnerHandle;

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
            let mut still_searching = true;
            while still_searching {
                use diesel::{self, ExpressionMethods, QueryDsl, RunQueryDsl, SelectableHelper};
                use id2202_autograder::db::schema::submissions::{
                    self, assigned_runner, exec_finished,
                };
                let ret: Result<Submission, _> = submissions::table
                    .select(Submission::as_select())
                    .filter(assigned_runner.eq(args.runner_id))
                    .filter(exec_finished.eq(false))
                    .first(&mut conn.conn);
                match ret {
                    Ok(sub) => {
                        log::warn!("Found unfinished submission: {sub:?}");
                        conn.github_comment_and_status(
                            &settings,
                            &sub,
                            &format!("{} {} {}",
                                "The runner was interrupted before it could finish grading your solution.",
                                "Please try to submit your solution again.",
                                "Contact course staff if the problem persists."
                            ),
                            SubmissionStatusCode::AutograderFailure,
                            true,
                        )?;
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
        let l = match Listener::from_settings(&notify_settings) {
            Ok(l) => l,
            Err(e) => {
                log::error!("Cannot initialize listener: {e}");
                return ();
            }
        };

        let mut watching = true;
        while watching {
            match l.listen() {
                Ok(res) => {
                    if !res.timedout {
                        // Received new event
                        msg_send.send(MSG_NOTIFY).unwrap_or_else({
                            |e| {
                                log::error!("Could not send notification message: {e}");
                                watching = false;
                            }
                        });
                    }
                }
                Err(e) => {
                    log::error!("Received error while watching file: {e}");
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
    let mut active_sub: Option<TestRunnerHandle> = None;

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
                run_handle.set_as_finished();
            }

            if run_handle.is_finished() {
                let res = run_handle.collect_results();

                match DatabaseConnection::connect(&settings) {
                    Ok(mut conn) => {
                        match conn.get_submission(run_handle.submission_id) {
                            Ok(sub) => {
                                // 1. First commit to the shadow repository.
                                let (markdown, status) = match commit_to_shadow(
                                    &settings,
                                    &sub,
                                    &run_handle.repo_dir,
                                    &run_handle.workspace,
                                    &res.shadow_files,
                                ) {
                                    Ok(()) => (res.gh_markdown, res.status),
                                    Err(e) => {
                                        // Not being able to record info in the
                                        // shadow repository is a fatal error,
                                        // as that is being used to check
                                        // whether a student has passed or not.
                                        log::error!(
                                            "Could not commit results to shadow repository: {e}"
                                        );
                                        let failmarkdown = format!(
                                            "**Error grading submission {}. Contact course staff.**",
                                            sub.id,
                                        );
                                        (failmarkdown, SubmissionStatusCode::AutograderFailure)
                                    }
                                };

                                // 2. Write a comment to the GitHub commit
                                conn.github_comment_and_status(
                                    &settings, &sub, &markdown, status, true,
                                )
                                .unwrap_or_else(|e| {
                                    log::warn!("Could not set commit message and/or status: {e}")
                                });
                                conn.set_exec_date_finished(sub.id).unwrap_or_else(|e| {
                                    log::warn!(
                                        "Could not set finish date for job {}: {}",
                                        sub.id,
                                        e
                                    )
                                });
                            }
                            Err(e) => {
                                log::warn!(
                                    "Error notifying results to user, could not find submission in database: {e}"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "Error notifying results to user, cannot connect to database: {e}"
                        );
                    }
                }

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

            match DatabaseConnection::connect(&settings) {
                Ok(mut conn) => {
                    match conn.try_assign_submission(args.runner_id) {
                        Ok(Some(sub)) => {
                            log::info!("Assigned submission: {:#?}", sub);
                            match TestRunnerHandle::new(&settings, &sub, args.runner_id) {
                                Ok(trh) => {
                                    active_sub = Some(trh);
                                    conn.github_comment_and_status(
                                        &settings,
                                        &sub,
                                        "The autograder is now running your submission. The results will be provided as a comment here when they are ready.",
                                        SubmissionStatusCode::Running,
                                        false,
                                    )
                                    .unwrap_or_else(|e| {
                                        log::warn!("Could not set commit message and/or status: {e}")
                                    });
                                    conn.set_exec_date_started(sub.id).unwrap_or_else(|e| {
                                        log::warn!(
                                            "Could not set start date for job {}: {}",
                                            sub.id,
                                            e
                                        )
                                    });
                                    // Do not wait for a timeout, just proceed
                                    // to run the test cases.
                                    next_offset = init_time.elapsed();
                                }
                                Err(commit_errmsg) => {
                                    conn.github_comment_and_status(
                                        &settings,
                                        &sub,
                                        &format!(
                                            "# Your submission could not be graded.\n\n{}",
                                            &commit_errmsg
                                        ),
                                        SubmissionStatusCode::SubmissionError,
                                        true,
                                    )
                                    .unwrap_or_else(|e| {
                                        log::warn!(
                                            "Could not set commit message and/or status: {e}"
                                        )
                                    });

                                    conn.set_exec_date_finished(sub.id).unwrap_or_else(|e| {
                                        log::warn!("Could not set exec finished: {e}")
                                    });
                                }
                            }
                        }
                        // No new job
                        Ok(None) => {}
                        Err(err) => {
                            log::warn!("Error checking for new jobs: {err:?}")
                        }
                    }
                }
                Err(err) => {
                    log::warn!("Error connecting to database: {err:?}")
                }
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
fn commit_to_shadow(
    settings: &Settings,
    sub: &Submission,
    repo_dir: &str,
    workspace_dir: &str,
    shadow_files: &Vec<(PathBuf, String)>,
) -> Result<(), Error> {
    // Async runtime for requests.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::from(format!("Could not unwrap tokio runtime: {e}")))?;

    // 1. Create the shadow repository if it does not exist.

    let shadow_name = format!("{}-shadow", &sub.github_repo);

    let shadow_exists =
        rt.block_on(async { github::repo_exists(settings, &sub.github_org, &shadow_name).await })?;
    if !shadow_exists {
        log::warn!("The shadow repository does not exist. Creating it.");
        rt.block_on(async {
            github::create_repo(settings, &sub.github_org, &shadow_name, true).await
        })?;
    }

    // 2. Clone the shadow repository.

    // Ensure that the workspace dir exists
    create_dir_if_not_exists(workspace_dir)?;

    // Set up necessary paths
    let shadow_dir = path_absolute_join(workspace_dir, "shadow")?;
    let shadow_addr = format!(
        "git@{}:{}/{}.git",
        &sub.github_address, &sub.github_org, &shadow_name,
    );

    let date_dir = path_absolute_join(
        &shadow_dir,
        systemtime_to_fsfriendly_utc_string(&sub.date_submitted)
            .ok_or(Error::from("Could not create date for date dir"))?,
    )?;
    let snapshot_dir = path_absolute_join(&shadow_dir, "snapshot")?;
    let snapshot_gitdir = path_absolute_join(&snapshot_dir, ".git")?;

    log::debug!("Cloning shadow directory {shadow_addr} to {shadow_dir}");
    syscommand_timeout(
        &["git", "clone", "--depth", "1", &shadow_addr, &shadow_dir],
        SyscommandSettings {
            expected_code: Some(0),
            ..Default::default()
        },
    )
    .inspect_err(|e| log::error!("Could not clone shadow repo {shadow_addr}: {e}"))?;

    syscommand_timeout(
        &[
            "git",
            "-C",
            &shadow_dir,
            "config",
            "--local",
            "user.name",
            "ID2202 Autograder",
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
    for (path, content) in shadow_files {
        let content_path = path_absolute_join(&date_dir, path)?;
        let mut f = std::fs::File::create(content_path)?;
        f.write(content.as_bytes())?;
    }

    // If snapshot dir exists, remove it
    if std::fs::exists(&snapshot_dir)? {
        std::fs::remove_dir_all(&snapshot_dir)?;
    }

    dircpy::copy_dir(&repo_dir, &snapshot_dir)?;

    // Remove the copied .git dir that should exist as well...
    if std::fs::exists(&snapshot_gitdir)? {
        std::fs::remove_dir_all(&snapshot_gitdir)?;
    }

    syscommand_timeout(
        &["git", "-C", &shadow_dir, "add", &date_dir, &snapshot_dir],
        SyscommandSettings {
            expected_code: Some(0),
            ..Default::default()
        },
    )
    .inspect_err(|e| log::error!("Could not add files to shadow repo {shadow_addr}: {e}"))?;

    let commit_msg = format!("Results for submission {}", sub.id);
    syscommand_timeout(
        &["git", "-C", &shadow_dir, "commit", "-m", &commit_msg],
        SyscommandSettings {
            expected_code: Some(0),
            ..Default::default()
        },
    )
    .inspect_err(|e| log::error!("Could not commit files to shadow repo {shadow_addr}: {e}"))?;

    syscommand_timeout(
        &["git", "-C", &shadow_dir, "push"],
        SyscommandSettings {
            expected_code: Some(0),
            ..Default::default()
        },
    )
    .inspect_err(|e| log::error!("Could not push files to shadow repo {shadow_addr}: {e}"))?;

    Ok(())
}
