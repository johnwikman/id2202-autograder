use clap::{Parser, Subcommand};
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
use std::ffi::OsString;
use std::sync::mpsc;
use subprocess::{Exec, Job};

use id2202_autograder::{
    config::Settings,
    config::{TestGroup, Tests},
    db::conn::DatabaseConnection,
    error::Error,
    podman,
    utils::systemtime_to_utc_string,
};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the TOML file containing the program settings
    #[arg(short, long)]
    settings: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Start {},
    ValidateSettings {
        /// Print out the title hierarchy of all test groups
        #[arg(short = 'T', long, default_value_t = false)]
        print_titles: bool,

        /// Print out the entire test configuration
        #[arg(short = 'C', long, default_value_t = false)]
        print_test_config: bool,
    },
    CheckDatabase {
        /// Fetch all submissions from the database
        #[arg(short = 'S', long, default_value_t = false)]
        all_submissions: bool,

        /// A runner to assign to jobs
        #[arg(long)]
        assign_runner: Option<i32>,
    },
    TestPodman {
        /// Test listing images
        #[arg(long = "images", default_value_t = false)]
        list_images: bool,

        /// Test listing networks
        #[arg(long = "networks", default_value_t = false)]
        list_networks: bool,

        /// Test listing networks
        #[arg(long = "ps", default_value_t = false)]
        list_containers: bool,
    },
    TestSyscommand {
        /// Test the cat command with the specific stdin
        #[arg(long = "stdin")]
        example_stdin: Option<String>,

        /// Test output with specified number of lines
        #[arg(long = "lines")]
        std_lines: Option<usize>,
    },
}

fn main() -> Result<(), Error> {
    let args: Args = Args::parse();
    let s = Settings::load(&args.settings)?;
    s.setup_logging("entrypoint")?;
    match args.command {
        Commands::Start {} => start(&args, &s),
        Commands::ValidateSettings {
            print_titles,
            print_test_config,
        } => validate_settings(s, print_titles, print_test_config),
        Commands::CheckDatabase {
            all_submissions,
            assign_runner,
        } => check_database(s, all_submissions, assign_runner),
        Commands::TestPodman {
            list_images,
            list_networks,
            list_containers,
        } => test_podman(s, list_images, list_networks, list_containers),
        Commands::TestSyscommand {
            example_stdin,
            std_lines,
        } => test_syscommand(s, example_stdin, std_lines),
    }
}

/// Starts the autograder, spawning the web API server process and the job
/// runner processes.
fn start(args: &Args, s: &Settings) -> Result<(), Error> {
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

    // Verify existence of podman image and networks
    log::debug!("Checking that the podman image exists");
    let podimgs = podman::images().unwrap();
    if !podimgs.contains(&s.runner.podman_image) {
        log::info!("Pulling the runner image {}", &s.runner.podman_image);
        podman::pull(&s.runner.podman_image).unwrap();
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
    let interval = std::time::Duration::from_secs(s.monitor.poll_interval_seconds.into());
    let mut next_offset = std::time::Duration::ZERO;

    // Functionality for interrupting on received signals
    let mut signals = Signals::new(&[SIGINT, SIGTERM])?;
    let (sigc_send, sigc_recv) = std::sync::mpsc::channel();
    let sigc_handle = std::thread::spawn(move || {
        for sig in signals.forever() {
            log::info!("Received signal {sig}");
            sigc_send
                .send("recvsig")
                .unwrap_or_else(|e| log::error!("Could not send notification message: {e}"));
            break;
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
                .args(&[
                    &OsString::from("--settings"),
                    &OsString::from(&args.settings),
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
                        &OsString::from(&args.settings),
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

/// Validates the loaded settings, used for printing them out
fn validate_settings(
    s: Settings,
    print_titles: bool,
    print_test_config: bool,
) -> Result<(), Error> {
    log::info!("VALIDATING SETTINGS");
    dbg!(&s);

    log::debug!("Loading test config");
    let tc = Tests::load(&s.runner.test_config)?;

    if print_test_config {
        log::debug!("Printing the entire test configuration");
        dbg!(&tc);
    };

    if print_titles {
        log::debug!("Printing the test configuration titles");
        fn recursively_print(tg: &TestGroup, indent: usize) {
            println!(
                "{} - {}",
                std::iter::repeat(" ").take(indent).collect::<String>(),
                &tg.title
            );
            for sg in tg.subgroups.iter() {
                recursively_print(sg, indent + 4);
            }
        }
        for (tagname, groups) in tc.tag_groups.iter() {
            println!("#{}", tagname);
            for tag in groups.iter() {
                for tg in tag.test_groups.iter() {
                    recursively_print(tg, 0);
                }
            }
        }
    };
    Ok(())
}

/// Checks the database connection
fn check_database(
    s: Settings,
    get_all_submissions: bool,
    assign_runner: Option<i32>,
) -> Result<(), Error> {
    use diesel::{self, ExpressionMethods, QueryDsl, RunQueryDsl, SelectableHelper};
    use id2202_autograder::db::conn::DatabaseConnection;

    log::info!("CHECKING DATABASE");

    log::debug!("Opening database connection");
    let mut dbconn = DatabaseConnection::connect(&s)?;

    if get_all_submissions {
        log::debug!("Fetching all submissions");
        use id2202_autograder::db::{
            models::Submission,
            schema::submissions::{self, id},
        };
        match submissions::table
            .select(Submission::as_select())
            .order(id.desc())
            .limit(100)
            .load(&mut dbconn.conn)
        {
            Ok(sub_vec) => {
                for sub in sub_vec.iter() {
                    let d = systemtime_to_utc_string(&sub.date_submitted)
                        .unwrap_or("NO_TIME".to_string());
                    println!("Date Submitted: {}\n{sub:#?}", d);
                }
            }
            Err(e) => {
                log::error!("Could not fetch all submissions: {e}")
            }
        }
    }
    if let Some(runner_id) = assign_runner {
        match dbconn.try_assign_submission(runner_id) {
            Ok(Some(sub)) => {
                println!("Assigned submission: {sub:#?}");
            }
            Ok(None) => {
                println!("No submission to assign");
            }
            Err(e) => {
                println!("Database error: {e}");
            }
        }
    }

    log::debug!("Done connecting");
    Ok(())
}

/// Test the notification on a specific file
fn test_podman(
    _s: Settings,
    list_images: bool,
    list_networks: bool,
    list_containers: bool,
) -> Result<(), Error> {
    if list_images {
        log::debug!("Listing images");
        match podman::images() {
            Ok(imgs) => println!("{:?}", imgs),
            Err(e) => println!("Could not list images: {e}"),
        }
    }

    if list_networks {
        log::debug!("Listing networks");
        match podman::networks() {
            Ok(nets) => println!("{:?}", nets),
            Err(e) => println!("Could not list networks: {e}"),
        }
    }

    if list_containers {
        log::debug!("Listing containers");
        match podman::ps_names() {
            Ok(names) => println!("{:?}", names),
            Err(e) => println!("Could not list containers: {e}"),
        }
    }

    Ok(())
}

/// Test the notification on a specific file
fn test_syscommand(
    _s: Settings,
    example_stdin: Option<String>,
    std_lines: Option<usize>,
) -> Result<(), Error> {
    use id2202_autograder::utils::{syscommand_timeout, SyscommandSettings};

    if let Some(s) = example_stdin {
        log::info!("Testing stdin for string \"{s}\"");
        match syscommand_timeout(
            ["bash", "-c", "cat"],
            SyscommandSettings {
                stdin: Some(s),
                max_stdout_length: Some(64 * 1024),
                max_stderr_length: Some(64 * 1024),
                ..Default::default()
            },
        ) {
            Ok(output) => println!("Got the following stdout back:\n\"{}\"", output.stdout),
            Err(e) => println!("Error running syscommand: {e}"),
        }
    }

    if let Some(lc) = std_lines {
        log::info!("Outputting {lc} lines to stdout");
        match syscommand_timeout(
            [
                "bash",
                "-c",
                &format!(
                    "for i in $(seq 1 {lc}); do echo {}; sleep 0.15; echo {} 1>&2; sleep 0.35; done",
                    "'(stdout) foo bar babar'", "'(stderr) foo bar babar'",
                ),
            ],
            SyscommandSettings {
                max_stdout_length: Some(64 * 1024),
                max_stderr_length: Some(64 * 1024),
                ..Default::default()
            },
        ) {
            Ok(output) => {
                println!("stdout ({} bytes):\n\"{}\"", output.stdout.len(), output.stdout);
                println!("stderr ({} bytes):\n\"{}\"", output.stderr.len(), output.stderr);
            }
            Err(e) => println!("Error running syscommand: {e}"),
        }
    }

    Ok(())
}
