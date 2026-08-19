use clap::{Parser, Subcommand};
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
use std::ffi::OsString;
use std::sync::mpsc;
use std::time::Duration;
use subprocess::{Exec, Job};

use id2202_autograder::{config::Settings, db::conn::DatabaseConnection, error::Error, podman};

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
    BuildImage(setup::BuildImageArgs),
    PullImage,
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
        Commands::BuildImage(a) => setup::build_image(s, a),
        Commands::PullImage => setup::pull_image(s),
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

    // Verify existence of podman image and networks. Images are never fetched
    // here: `pull-image` and `build-image` do that, so that starting the
    // autograder does not depend on the network.
    log::debug!("Checking that the podman images exist");
    let podimgs = podman::images().unwrap();
    for (image, how) in
        [(&s.runner.podman_image, "pull-image"), (&s.runner.podman_verifier_image, "build-image")]
    {
        if !podimgs.contains(image) {
            return Err(Error::runtime(format!(
                "the podman image \"{image}\" does not exist, run `entrypoint {how}` first"
            )));
        }
    }
    log::debug!("Checking that the podman networks exists for each runner");
    let podnets = podman::networks().unwrap();
    for runner_id in 0..s.runner.n_runners {
        let expected_net = format!("{}{}", s.runner.podman_network_prefix, runner_id);
        if !podnets.contains(&expected_net) {
            podman::create_network(&expected_net).unwrap();
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
