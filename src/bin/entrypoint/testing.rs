//! Commands for inspecting a deployment: printing what was loaded and poking
//! at the things the autograder depends on. None of them change any state.

use clap::Args;

use id2202_autograder::{
    config::{Settings, TestGroup, Tests, TestsLoadingOptions},
    error::Error,
    podman::{self, SHARED},
    utils::utc_string,
};

#[derive(Args, Debug)]
pub struct ValidateSettingsArgs {
    /// Print out the title hierarchy of all test groups
    #[arg(short = 'T', long, default_value_t = false)]
    pub print_titles: bool,

    /// Print out the entire test configuration
    #[arg(short = 'C', long, default_value_t = false)]
    pub print_test_config: bool,
}

/// Validates the loaded settings, used for printing them out
pub fn validate_settings(s: Settings, args: ValidateSettingsArgs) -> Result<(), Error> {
    log::info!("VALIDATING SETTINGS");
    dbg!(&s);

    log::debug!("Loading test config");
    let tc = Tests::load(&s, &s.runner.test_config, TestsLoadingOptions::default())?;

    if args.print_test_config {
        log::debug!("Printing the entire test configuration");
        dbg!(&tc);
    };

    if args.print_titles {
        log::debug!("Printing the test configuration titles");
        fn recursively_print(tg: &TestGroup, indent: usize) {
            println!("{} - {}", std::iter::repeat_n(" ", indent).collect::<String>(), tg.title);
            for sg in tg.subgroups.iter() {
                recursively_print(sg, indent + 4);
            }
        }
        for (tagname, tag) in tc.tags.iter() {
            println!("#{}", tagname);
            for tg in tag.test_groups.iter() {
                recursively_print(tg, 0);
            }
        }
    };
    Ok(())
}

/// Checks, without starting or changing anything, that this system provides
/// what the autograder needs from it. Prints one line per check. Returns an
/// error if at least one system check fails.
pub fn validate_system(_s: &Settings) -> Result<(), Error> {
    use std::{
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::Path,
    };

    let mut failed_checks = 0;
    let mut check = |cond: bool, what: &str| {
        println!(
            "{} {}",
            if cond { "\x1b[1;32mPASS\x1b[0m" } else { "\x1b[1;31mFAIL\x1b[0m" },
            what
        );
        if !cond {
            failed_checks += 1;
        }
    };

    // helper functions
    fn executable(path: &Path) -> bool {
        path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    fn on_path(prog: &str) -> bool {
        std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|dir| executable(&dir.join(prog))))
    }

    for bin in ["git", "ssh", "file", "podman", "mkfs.ext4", "fuse2fs", "fusermount3", "df"] {
        check(on_path(bin), &format!("\x1b[3;33m{bin}\x1b[0m is on PATH"));
    }
    if !std::fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0) {
        let fusermount = std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path).map(|dir| dir.join("fusermount3")).find(|p| executable(p))
        });
        check(
            fusermount
                .and_then(|p| p.metadata().ok())
                .is_some_and(|m| m.uid() == 0 && m.mode() & 0o4000 != 0),
            "\x1b[3;33mfusermount3\x1b[0m is setuid root",
        );
    }
    check(
        executable(Path::new(podman::FUSE_OVERLAYFS)),
        &format!("{} is executable", podman::FUSE_OVERLAYFS),
    );

    check(std::fs::read_to_string("/proc/mounts").is_ok(), "/proc/mounts is readable");

    check(
        std::fs::exists("/sys/fs/cgroup/cgroup.controllers").unwrap_or(false),
        "/sys/fs/cgroup is a cgroup v2 hierarchy",
    );
    let fuse = std::fs::OpenOptions::new().read(true).write(true).open("/dev/fuse");
    check(fuse.is_ok(), "/dev/fuse opens for reading and writing");
    drop(fuse);

    match SHARED.info() {
        Ok(info) => {
            let host = &info.host;
            check(
                host.cgroup_version == "v2",
                &format!("podman uses cgroup {}", host.cgroup_version),
            );
            for controller in ["memory", "pids"] {
                check(
                    host.cgroup_controllers.iter().any(|c| c == controller),
                    &format!("the {controller} controller is available to podman"),
                );
            }
            check(
                host.network_backend == "netavark",
                &format!("podman's network backend is {}", host.network_backend),
            );
            let backend = &host.network_backend_info.path;
            check(
                executable(Path::new(backend)),
                &format!("podman's network backend {backend} is executable"),
            );
            if host.security.rootless {
                let pasta = &host.pasta.executable;
                check(
                    executable(Path::new(pasta)),
                    &format!("pasta {pasta} is executable for rootless networking"),
                );

                let uids: usize = host.id_mappings.uidmap.iter().flatten().map(|m| m.size).sum();
                let gids: usize = host.id_mappings.gidmap.iter().flatten().map(|m| m.size).sum();
                check(uids > 1, &format!("podman's rootless uidmap maps {uids} ids"));
                check(gids > 1, &format!("podman's rootless gidmap maps {gids} ids"));

                let tun = std::fs::OpenOptions::new().read(true).write(true).open("/dev/net/tun");
                check(tun.is_ok(), "/dev/net/tun opens for rootless networking");
            }
        }
        Err(e) => check(false, &format!("`podman info` runs: {e}")),
    }

    if failed_checks == 0 {
        Ok(())
    } else {
        Error::err_runtime(format!("{failed_checks} failed system checks"))
    }
}

#[derive(Args, Debug)]
pub struct CheckDatabaseArgs {
    /// Fetch all submissions from the database
    #[arg(short = 'S', long, default_value_t = false)]
    pub all_submissions: bool,

    /// A runner to assign to jobs
    #[arg(long)]
    pub assign_runner: Option<i32>,
}

/// Checks the database connection
pub fn check_database(s: Settings, args: CheckDatabaseArgs) -> Result<(), Error> {
    use diesel::{self, ExpressionMethods, QueryDsl, RunQueryDsl, SelectableHelper};
    use id2202_autograder::db::conn::DatabaseConnection;

    log::info!("CHECKING DATABASE");

    log::debug!("Opening database connection");
    let mut dbconn = DatabaseConnection::connect(&s)?;

    if args.all_submissions {
        log::debug!("Fetching all submissions");
        use id2202_autograder::db::{
            models::{raw::SubmissionRow, Submission},
            schema::submissions::{self, id},
        };
        let rows: Result<Vec<SubmissionRow>, _> = submissions::table
            .select(SubmissionRow::as_select())
            .order(id.desc())
            .limit(100)
            .load(&mut dbconn.conn);
        let subs: Result<Vec<Submission>, Error> =
            rows.map_err(Error::from).and_then(|rows| Submission::from_rows(&mut dbconn, rows));
        match subs {
            Ok(sub_vec) => {
                for sub in sub_vec.iter() {
                    println!("Date Submitted: {}\n{sub:#?}", utc_string(&sub.submitted_at));
                }
            }
            Err(e) => {
                log::error!("Could not fetch all submissions: {e}")
            }
        }
    }
    if let Some(runner_id) = args.assign_runner {
        match dbconn.try_claim_submission(runner_id) {
            Ok(Some(claim)) => {
                println!("Claimed submission: {:#?}", claim.claimed);
                if !claim.deferred.is_empty() {
                    println!("Jobs left unclaimed: {:#?}", claim.deferred);
                }
            }
            Ok(None) => {
                println!("No submission to claim");
            }
            Err(e) => {
                println!("Database error: {e}");
            }
        }
    }

    log::debug!("Done connecting");
    Ok(())
}

#[derive(Args, Debug)]
pub struct TestPodmanArgs {
    /// Test listing images
    #[arg(long = "images", default_value_t = false)]
    pub list_images: bool,

    /// Test listing networks
    #[arg(long = "networks", default_value_t = false)]
    pub list_networks: bool,

    /// Test listing networks
    #[arg(long = "ps", default_value_t = false)]
    pub list_containers: bool,
}

/// Test the notification on a specific file
pub fn test_podman(_s: Settings, args: TestPodmanArgs) -> Result<(), Error> {
    if args.list_images {
        log::debug!("Listing images");
        match podman::images() {
            Ok(imgs) => println!("{:?}", imgs),
            Err(e) => println!("Could not list images: {e}"),
        }
    }

    if args.list_networks {
        log::debug!("Listing networks");
        match podman::network_names(&podman::SHARED) {
            Ok(nets) => println!("{:?}", nets),
            Err(e) => println!("Could not list networks: {e}"),
        }
    }

    if args.list_containers {
        log::debug!("Listing containers");
        match podman::ps_names(&podman::SHARED) {
            Ok(names) => println!("{:?}", names),
            Err(e) => println!("Could not list containers: {e}"),
        }
    }

    Ok(())
}

#[derive(Args, Debug)]
pub struct TestSyscommandArgs {
    /// Test the cat command with the specific stdin
    #[arg(long = "stdin")]
    pub example_stdin: Option<String>,

    /// Test output with specified number of lines
    #[arg(long = "lines")]
    pub std_lines: Option<usize>,
}

/// Test the notification on a specific file
pub fn test_syscommand(_s: Settings, args: TestSyscommandArgs) -> Result<(), Error> {
    use id2202_autograder::utils::{syscommand_timeout, SyscommandSettings};

    if let Some(s) = args.example_stdin {
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

    if let Some(lc) = args.std_lines {
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
