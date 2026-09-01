use clap::Parser;
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use id2202_autograder::{
    config::Settings,
    db::{
        conn::DatabaseConnection,
        models::{Submission, SubmissionJobPlain, SubmissionStatus},
        notify::listen as db_listen,
    },
    error::Error,
    reporting::{
        structured_text::StructuredParagraph, MetaJobResultsReport, MetaReport, Report,
        ReportMessage, ReportWrapper,
    },
};

mod shadow;
mod subrunner;
use subrunner::SubmissionRunnerHandle;

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

const MSG_NOTIFY: &str = "notify";
const MSG_SIGNAL: &str = "signal";

fn main() -> Result<(), Error> {
    let args: Args = Args::parse();
    let settings = Settings::load(&args.settings)?;

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;

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
            let abandoned: Vec<Submission> =
                Submission::assigned_to_runner(&mut conn, args.runner_id)?;
            for mut sub in abandoned {
                log::warn!("Found submission {} with jobs left unfinished", sub.id);

                SubmissionJobPlain::abandon_all(
                    sub.jobs.iter_mut().filter(|j| j.is_claimed_by(args.runner_id)),
                    &mut conn,
                    &err_report,
                )?;

                // This may fail as the submission origin could be unavailable
                rt.block_on(sub.origin.set_status_and_report(
                    &settings,
                    &MetaReport::Transient(&err_report),
                    sub.status(),
                ))
                .unwrap_or_else(|e| {
                    log::warn!(
                        "Could not set status and report for abandoned submission {}: {e}",
                        sub.id
                    );
                });
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
    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    let sigc_send = msg_send.clone();
    let sigc_handle = std::thread::spawn(move || {
        if let Some(sig) = signals.forever().next() {
            log::info!("Received signal {sig}");
            sigc_send
                .send(MSG_SIGNAL)
                .unwrap_or_else(|e| log::error!("Could not send notification message: {e}"));
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
                run_handle
                    .set_as_erroneous()
                    .inspect_err(|e| log::error!("Could not set run_handle as erroneous: {e}"))?;
            }

            if run_handle.is_finished() {
                // At this point the results should have been recorded to the
                // shadow repository and to the database as each tag finished.
                // Now we just have to send back the result report to the
                // origin.
                let results = run_handle.job_results();

                // A claimed job only covers the jobs that were eligible for
                // it. So we do a read-back of the submission from the database
                // to get the full submission status.
                let mut conn = DatabaseConnection::connect(&settings)?;
                let sub: Submission = Submission::by_id(&mut conn, run_handle.submission_id())?;

                rt.block_on(async {
                    run_handle
                        .origin()
                        .set_status_and_report(
                            &settings,
                            &MetaReport::JobResults(MetaJobResultsReport { jobs: &results }),
                            sub.status(),
                        )
                        .await
                })
                .unwrap_or_else(|e| log::warn!("Could not set commit message and/or status: {e}"));

                log::info!("Grading of submission {} done.", run_handle.submission_id());
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

            if let Some(claim) = conn.try_claim_submission(args.runner_id)? {
                log::info!("Claimed submission: {:#?}", claim.claimed);

                // Kept because the handle takes ownership of the submission,
                // and the error path still has to report back to the student.
                let mut claimed = claim.claimed.clone();
                let start_msg = claim_message(&claimed.jobs, &claim.deferred);

                match SubmissionRunnerHandle::new(&settings, claim.claimed, args.runner_id) {
                    Ok(handle) => {
                        // Posted as markdown rather than as a report: the tag
                        // names in it are code spans, which the report
                        // escaper would show as literal backticks.
                        rt.block_on(claimed.origin.set_status_and_report(
                            &settings,
                            &MetaReport::Structured(start_msg),
                            SubmissionStatus::InProgress,
                        ))
                        .unwrap_or_else(|e| {
                            log::warn!("Could not set commit message and/or status: {e}")
                        });

                        active_sub = Some(handle);
                        // Do not wait for a timeout, just proceed
                        // to run the test cases.
                        next_offset = init_time.elapsed();
                    }
                    Err(report) => {
                        // Nothing was graded: the handle failed before any job
                        // could start, so these are voided rather than failed.
                        SubmissionJobPlain::abandon_all(&mut claimed.jobs, &mut conn, &report)?;

                        // Ensure that the commit status is still _waiting_ if
                        // there are more jobs to run later for this
                        // submission.
                        let status = match claim.deferred.is_empty() {
                            true => SubmissionStatus::Failed,
                            false => SubmissionStatus::Waiting,
                        };

                        rt.block_on(claimed.origin.set_status_and_report(
                            &settings,
                            &MetaReport::Transient(&Report::Wrapper(ReportWrapper {
                                title: Some("Your submission could not be graded.".to_string()),
                                reports: vec![report],
                            })),
                            status,
                        ))
                        .unwrap_or_else(|e| {
                            log::warn!("Could not set commit message and/or status: {e}")
                        });
                    }
                }
            }

            let sleep_time = next_offset.checked_sub(init_time.elapsed()).unwrap_or(Duration::ZERO);
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

/// The comment posted when a runner starts grading, naming the tags of
/// this run and any that are being held back for a later one.
fn claim_message<'a>(
    claimed: &[SubmissionJobPlain],
    deferred: &[SubmissionJobPlain],
) -> StructuredParagraph<'a> {
    use id2202_autograder::reporting::structured_text::{
        StructuredInline as Inline, StructuredParagraph as Par,
    };

    let mut paragraphs: Vec<Par> = vec![];

    paragraphs.push(Par::Paragraph(Inline::Sentences(vec![
        if deferred.is_empty() {
            Inline::plain_str("The autograder is now grading your submission")
        } else {
            Inline::sep_space(vec![
                Inline::plain_str("The autograder is now grading your submission for"),
                Inline::OxfordCommaSepWords(
                    claimed.iter().map(|job| Inline::inline_code(job.tag.clone())).collect(),
                ),
            ])
        },
        Inline::plain_str("The results will be provided as a comment here when they are ready."),
    ])));

    if !deferred.is_empty() {
        paragraphs.push(Par::Paragraph(Inline::Sentences(vec![Inline::sep_space(vec![
            Inline::plain_str("Your submission for"),
            Inline::OxfordCommaSepWords(
                deferred.iter().map(|job| Inline::inline_code(job.tag.clone())).collect(),
            ),
            Inline::plain_str(if deferred.len() == 1 { "is" } else { "are" }),
            Inline::plain_str("rate-limited and will be run later."),
        ])])));
    }

    Par::Many(paragraphs)
}
