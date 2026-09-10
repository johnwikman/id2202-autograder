use clap::{Parser, Subcommand};
use diesel::{ExpressionMethods, QueryDsl, RunQueryDsl};
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
use std::time::{Duration, SystemTime};
use std::{collections::BTreeSet, sync::mpsc};
use std::{ffi::OsString, path::Path};
use subprocess::{Exec, Job};

use id2202_autograder::{
    config::Settings,
    db::{
        conn::DatabaseConnection,
        models::{Submission, SubmissionJobPlain},
    },
    error::Error,
    podman,
    reporting::{MetaReport, Report, ReportMessage},
    utils::path_absolute_join,
};

mod setup;
mod testing;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the TOML file containing the program settings
    #[arg(short, long, global = true)]
    settings: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Start,
    ValidateSettings(testing::ValidateSettingsArgs),
    CheckDatabase(testing::CheckDatabaseArgs),
    TestPodman(testing::TestPodmanArgs),
    TestSyscommand(testing::TestSyscommandArgs),
    SetupImages(setup::SetupImageArgs),
    VerifySshHosts(setup::VerifySshHostsArgs),
}

fn main() -> Result<(), Error> {
    let args: Args = Args::parse();
    // Global rather than a plain option, so it may appear on either side of the
    // subcommand. clap forbids `required` on a global argument, so the check is
    // here instead.
    let settings = args
        .settings
        .as_deref()
        .ok_or_else(|| Error::runtime("missing required option --settings <SETTINGS>"))?;
    let s = Settings::load(settings)?;
    s.setup_logging("entrypoint")?;
    match args.command {
        Commands::Start => start(&s),
        Commands::ValidateSettings(a) => testing::validate_settings(s, a),
        Commands::CheckDatabase(a) => testing::check_database(s, a),
        Commands::TestPodman(a) => testing::test_podman(s, a),
        Commands::TestSyscommand(a) => testing::test_syscommand(s, a),
        Commands::SetupImages(a) => setup::setup_images(s, a),
        Commands::VerifySshHosts(a) => setup::verify_ssh_hosts(s, a),
    }
}

/// Starts the autograder, spawning the web API server process and the job
/// runner processes.
fn start(s: &Settings) -> Result<(), Error> {
    let entrypoint_bin = std::env::current_exe()?;
    let binary_dir = entrypoint_bin
        .parent()
        .ok_or_else(|| Error::runtime("could not get parent of the entrypoint binary"))?
        .canonicalize()?;
    let server_bin = binary_dir.join("server");
    let runner_bin = binary_dir.join("runner");
    log::debug!("Entrypoint binary: {}", entrypoint_bin.to_str().unwrap());
    log::debug!("Server binary: {}", server_bin.to_str().unwrap());
    log::debug!("Runner binary: {}", runner_bin.to_str().unwrap());

    // Ensure that runtime directories exist before proceeding.
    let crucial_dirs: Vec<&str> = vec![
        &s.log.dir,
        &s.runner.shadow_dir,
        &s.runner.workspace_dir,
        &s.submission.direct.storage_dir,
    ];
    for path in crucial_dirs {
        let path = Path::new(path);
        if !path.exists() {
            std::fs::create_dir_all(path).map_err(|e| {
                Error::fs("could not create directory", path.to_string_lossy()).with_cause(e)
            })?;
        } else if !path.is_dir() {
            return Error::err_fs(
                "expected crucial directory, found something else",
                path.to_string_lossy(),
            );
        }
    }

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

    // Verify existence of podman image and networks. Images are never fetched
    // here: `pull-image` and `build-image` do that, so that starting the
    // autograder does not depend on the network.
    log::debug!("Checking that the podman images exist");
    let podimgs = podman::images().unwrap();
    for (name, img) in &s.runner.podman.images {
        if !podimgs.contains(&img.image) {
            return Err(Error::runtime(format!(
                "Missing image \"{}\" for {}. Run `entrypoint setup-images`.",
                img.image, name,
            )));
        }
    }

    log::debug!("Ensuring that the podman networks exists for each runner");
    let podnets = podman::networks().unwrap();
    for runner_id in 0..s.runner.n_runners {
        let expected_net = format!("{}{}", s.runner.podman.network_prefix, runner_id);
        if !podnets.contains(&expected_net) {
            podman::create_network(&expected_net).unwrap();
        }
    }

    // Clean up any pending submissions that will never be picked up by a
    // runner. (The scope is to ensure drop of conn)
    {
        let mut conn = DatabaseConnection::connect(s)?;
        let report = Report::Message(ReportMessage {
            msg: format!(
                "{} {} {}",
                "The runner was interrupted before it could finish grading your solution.",
                "Please try to submit your solution again.",
                "Contact course staff if the problem persists."
            ),
        });
        let retired: Vec<Submission> =
            Submission::assigned_to_retired_runners(&mut conn, s.runner.n_runners)?;
        for mut sub in retired {
            log::warn!("Submission {} has jobs held by a retired runner", sub.id);

            SubmissionJobPlain::abandon_all(
                sub.jobs.iter_mut().filter(|job| {
                    job.terminal_at().is_none()
                        && job.assigned_runner_id.is_some_and(|id| {
                            usize::try_from(id).is_ok_and(|id| id >= s.runner.n_runners)
                        })
                }),
                &mut conn,
                &report,
            )?;

            rt.block_on(sub.origin.set_status_and_report(
                s,
                &MetaReport::Transient(&report),
                sub.status(),
                Some(sub.id),
            ))?;
        }
    }

    // Using the .take() function to set these to None in the loop
    let mut proc_handle_server: Option<Job> = None;
    let mut proc_handles_runner: Vec<Option<Job>> = vec![];
    for _ in 0..s.runner.n_runners {
        proc_handles_runner.push(None); // a .init function would be nicer...
    }

    let init_time = std::time::Instant::now();
    let interval = Duration::from_secs(s.monitor.poll_interval_seconds.into());
    let mut next_offset = Duration::ZERO;

    // Functionality for interrupting on received signals
    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    let (sigc_send, sigc_recv) = std::sync::mpsc::channel();
    let sigc_handle = std::thread::spawn(move || {
        if let Some(sig) = signals.forever().next() {
            log::info!("Received signal {sig}");
            sigc_send
                .send("recvsig")
                .unwrap_or_else(|e| log::error!("Could not send notification message: {e}"));
        }
    });

    let mut running = true;
    while running {
        next_offset += interval;
        log::debug!("Checking if binaries are still running");
        if let Some(exitstat_server) = proc_handle_server.as_ref().and_then(Job::poll) {
            log::error!("server process ended prematurely with exit status {exitstat_server:?}");
            proc_handle_server.take();
        }
        for handle_runner in proc_handles_runner.iter_mut() {
            if let Some(exitstat_runner) = handle_runner.as_ref().and_then(Job::poll) {
                log::error!(
                    "runner process ended prematurely with exit status {exitstat_runner:?}"
                );
                handle_runner.take();
            }
        }

        if proc_handle_server.is_none() {
            log::info!("Spawning a new server process");
            match Exec::cmd(server_bin.as_os_str())
                .args([
                    &OsString::from("--settings"),
                    &OsString::from(&s.origin_path),
                    &OsString::from("serve"),
                ])
                .start()
            {
                Ok(proc) => {
                    proc_handle_server = Some(proc);
                }
                Err(popen_err) => {
                    log::error!("Could not start server process: {popen_err}");
                }
            }
        }
        for (i, handle_runner) in proc_handles_runner.iter_mut().enumerate() {
            if handle_runner.is_none() {
                log::info!("Spawning a new runner process (ID: {i})");
                match Exec::cmd(runner_bin.as_os_str())
                    .args([
                        &OsString::from("--settings"),
                        &OsString::from(&s.origin_path),
                        &OsString::from("--runner-id"),
                        &OsString::from(i.to_string()),
                    ])
                    .start()
                {
                    Ok(job) => {
                        // We know that the previous value is None
                        #[allow(unused)]
                        handle_runner.insert(job);
                    }
                    Err(popen_err) => {
                        log::error!("Could not start runner (index={i}) process: {popen_err}");
                    }
                }
            }
        }

        log::debug!("Housekeeping on direct submission files");
        if let Err(e) = housekeep_storage_dir(s) {
            log::error!("Could not perform housekeeping on the storage directory: {e}");
            running = false;
        }

        if running {
            let sleep_time = next_offset - init_time.elapsed();
            match sigc_recv.recv_timeout(sleep_time) {
                Ok(_) => {
                    // Received a message on the signal channel, no longer running
                    running = false;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {} // timeout, expected
                Err(e) => {
                    log::warn!("Received unexpected channel error: {e}")
                }
            }
        }
        if running && sigc_handle.is_finished() {
            log::error!("Signal handler finished unexpectedly.");
            running = false
        }
    }
    log::info!("Stopping any child processes");
    if let Some(proc_server) = proc_handle_server.as_mut() {
        if let Err(e) = proc_server.terminate() {
            log::warn!("Got error {e} when terminating the server process");
        }
    }
    for (i, handle_runner) in proc_handles_runner.iter_mut().enumerate() {
        if let Some(Err(e)) = handle_runner.as_ref().map(Job::terminate) {
            log::warn!("Got error {e} when terminating the runner (index={i}) process");
        }
    }

    // Also notify listeners in the database, as some runner threads may still
    // be waiting for notifications on this channel.
    DatabaseConnection::connect(s)
        .and_then(|mut conn| conn.notify("submission"))
        .unwrap_or_else(|e| log::warn!("Could not notify: {e:#}"));

    log::info!("Entrypoint process exiting");
    Ok(())
}

/// Performs housekeeping on the storage directory, cleaning up old files no
/// longer needed for direct submissions.
fn housekeep_storage_dir(s: &Settings) -> Result<(), Error> {
    use id2202_autograder::db::schema::{
        submission_info_direct::{self, columns as d_info_col},
        submission_jobs::{self, columns as job_col},
    };
    let mut files: BTreeSet<String> = BTreeSet::new();
    for entry in std::fs::read_dir(&s.submission.direct.storage_dir)? {
        let entry = entry?;

        // Only check .tar.gz files
        let path = path_absolute_join(&s.submission.direct.storage_dir, entry.path())?;
        if !path.ends_with(".tar.gz") {
            continue;
        }

        // Ignore files not older than 15 seconds
        let Ok(file_age) = SystemTime::now().duration_since(entry.metadata()?.modified()?) else {
            log::warn!("Could not determine file age of {path}");
            continue;
        };
        if file_age < Duration::from_secs(15) {
            continue;
        }

        files.insert(path);
    }

    // No files to perform housekeeping on.
    if files.is_empty() {
        return Ok(());
    }

    let mut conn = DatabaseConnection::connect(s)?;
    let check_active: Vec<(i64, String, bool)> = submission_info_direct::table
        .select((
            d_info_col::submission_id,
            d_info_col::local_path,
            d_info_col::submission_id.eq_any(
                submission_jobs::table
                    .select(job_col::submission_id)
                    .filter(job_col::finished_at.is_null())
                    .filter(job_col::voided_at.is_null()),
            ),
        ))
        .filter(d_info_col::local_path.eq_any(&files))
        .load(&mut conn.conn)?;

    for (submission_id, local_path, in_use) in &check_active {
        if !in_use && std::fs::exists(local_path)? {
            log::info!("Removing submitted code for submission {submission_id}: {local_path}");
            std::fs::remove_file(local_path)?;
            files.remove(local_path);
        }
    }

    // Remove files which have no submission associated with them at all
    let known_files: BTreeSet<&str> = check_active.iter().map(|x| x.1.as_str()).collect();
    for local_path in files.iter().filter(|path| !known_files.contains(path.as_str())) {
        if std::fs::exists(local_path)? {
            log::info!("Removing orphaned code (without a submission): {local_path}");
            std::fs::remove_file(local_path)?;
        }
    }

    Ok(())
}
