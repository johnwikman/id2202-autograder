use serde::Deserialize;
use toml;

use crate::error::Error;
use crate::utils::{path_absolute_join, path_absolute_parent, path_join};

#[derive(Deserialize, Debug, Clone)]
pub struct Settings {
    pub name: String,

    /// The directory in which we can put temporary files.
    pub temp_dir: String,

    pub log: LoggingSettings,
    pub monitor: MonitorSettings,
    pub notify: NotifySettings,
    pub github: GitHubSettings,
    pub postgres: PostgresSettings,
    pub server: ServerSettings,
    pub runner: RunnerSettings,

    #[serde(skip)]
    pub reldir: String,
}

/// Logging settings
#[derive(Deserialize, Debug, Clone)]
pub struct LoggingSettings {
    /// Directory where to store log messages
    pub dir: String,

    /// Whether to output debug messages.
    pub verbose: bool,
}

/// Settings for the entrypoint monitor loop
#[derive(Deserialize, Debug, Clone)]
pub struct MonitorSettings {
    /// The frequency at which child processes are polled for whether they are
    /// alive or not.
    pub poll_interval_seconds: u16,
}

/// Settings for process notification
#[derive(Deserialize, Debug, Clone)]
pub struct NotifySettings {
    /// The file used to notify other processes on. This is a broadcast
    /// notification that will signal other processes to do something as soon
    /// as something is written to this file.
    pub path: String,

    /// Timeout for polling the notification file, to make sure that a process
    /// does not freeze due to polling.
    pub poll_timeout_millisec: u16,
}

/// Settings specific to incoming GitHub requests. See `ServerSettings` for
/// generic HTTP settings that applies to all incoming requests.
#[derive(Deserialize, Debug, Clone)]
pub struct GitHubSettings {
    /// The IP or domain address at which the GitHub instance is hosted at
    pub address: String,

    /// GitHub organization to accept grading requests from
    pub org: String,

    /// Allow submissions from other GitHub organizations
    pub allow_any_org: bool,

    /// Allowed repository prefixes: If not empty, a repository must start with
    /// one of these prefix strings to be graded.
    pub allowed_repo_prefixes: Vec<String>,

    /// Allowed repository suffixes: If not empty, a repository must end with
    /// one of these suffix strings to be graded.
    pub allowed_repo_suffixes: Vec<String>,

    /// A repository is not allowed to start with one of these strings to be
    /// graded.
    pub prohibited_repo_prefixes: Vec<String>,

    /// A repository is not allowed to end with one of these strings to be
    /// graded.
    pub prohibited_repo_suffixes: Vec<String>,

    /// GitHub authorization token for using the API
    pub auth_token: String,

    /// Webhook secret used to validate incoming requests
    pub webhook_secret: String,

    /// Maximum size of JSON payload
    pub max_payload: usize,

    /// A signature to place at the end of every comment made on GitHub
    pub comment_signature: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PostgresSettings {
    pub user: String,
    pub password: String,
    pub host: String,
    pub port: u16,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ServerSettings {
    pub address: String,
    pub port: u16,
}

/// Settings for runner processes
#[derive(Deserialize, Debug, Clone)]
pub struct RunnerSettings {
    /// How many runners to spawn
    pub n_runners: usize,

    /// How frequently that a runner should poll the database, ignoring any
    /// notifications
    pub database_poll_interval_seconds: u16,

    /// The docker/podman image to use for grading
    pub podman_image: String,

    /// The prefix to use for the network attached to the image. The network
    /// will be named as "{prefix}{runner_id}"
    pub podman_network_prefix: String,

    /// The directory inside the container where the repository will be mounted
    pub mount_repo: String,

    /// The directory inside the container where a test case will be located
    pub mount_tests: String,

    /// Directory to use as a workspace, to store temporary files
    pub workspace_dir: String,

    /// Path to the test configuration
    pub test_config: String,

    /// Markdown output settings
    pub md_settings: RunnerMarkdownSettings,
}

/// Settings for runner processes
#[derive(Deserialize, Debug, Clone)]
pub struct RunnerMarkdownSettings {
    /// Symbol used to indicate the success of a test case, or a group of test
    /// cases.
    pub symbol_ok: String,

    /// Symbol used to indicate that a test or group of tests were not run.
    pub symbol_skipped: String,

    /// Symbol used to indicate that a test or group of tests failed.
    pub symbol_failed: String,

    /// Celebratory symbol used when a tag has successfully passed.
    pub symbol_tagsuccess: String,

    /// Symbol used to represent a build stage.
    pub symbol_build: String,
}

impl Settings {
    /// Loads settings from the specified path
    pub fn load(path: &str) -> Result<Self, Error> {
        let contents: String = std::fs::read_to_string(path)
            .inspect_err(|e| eprintln!("Could not load settings from \"{path}\": {e}"))?;
        let mut s: Settings = toml::from_str(&contents)
            .inspect_err(|e| eprintln!("Error parsing settings from \"{path}\": {e}"))?;

        //eprintln!("Setting up canonical dir that the settings file is located in");
        s.reldir = path_absolute_parent(&path)?;

        //eprintln!("Converting relative paths to absolute paths");
        s.log.dir = path_absolute_join(&s.reldir, &s.log.dir)?;
        s.runner.workspace_dir = path_absolute_join(&s.reldir, &s.runner.workspace_dir)?;
        s.runner.test_config = path_absolute_join(&s.reldir, &s.runner.test_config)?;

        //eprintln!("Checking for environment variable overrides");
        if let Ok(log_dir) = std::env::var("AUTOGRADER_LOG_DIR") {
            s.log.dir = log_dir;
        }
        if let Ok(truth_value) = std::env::var("AUTOGRADER_LOG_VERBOSE") {
            s.log.verbose = match truth_value.to_lowercase().as_str() {
                "true" | "t" | "yes" | "y" => true,
                _ => false,
            };
        }

        // GitHub
        if let Ok(auth_token) = std::env::var("AUTOGRADER_GITHUB_AUTH_TOKEN") {
            s.github.auth_token = auth_token;
        }
        if let Ok(webhook_secret) = std::env::var("AUTOGRADER_GITHUB_WEBHOOK_SECRET") {
            s.github.webhook_secret = webhook_secret;
        }

        // Postgres
        if let Ok(user) = std::env::var("AUTOGRADER_POSTGRES_USER") {
            s.postgres.user = user;
        }
        if let Ok(password) = std::env::var("AUTOGRADER_POSTGRES_PASSWORD") {
            s.postgres.password = password;
        }
        if let Ok(host) = std::env::var("AUTOGRADER_POSTGRES_HOST") {
            s.postgres.host = host;
        }
        if let Ok(port) = std::env::var("AUTOGRADER_POSTGRES_PORT") {
            s.postgres.port = port
                .parse()
                .map_err(|e| Error::from(format!("Invalid postgres port value \"{port}\": {e}")))?;
        }

        // Server
        if let Ok(address) = std::env::var("AUTOGRADER_SERVER_ADDRESS") {
            s.server.address = address;
        }
        if let Ok(port) = std::env::var("AUTOGRADER_SERVER_PORT") {
            s.server.port = port
                .parse()
                .map_err(|e| Error::from(format!("Invalid server port value \"{port}\": {e}")))?;
        }

        // Runner
        if let Ok(n_runners) = std::env::var("AUTOGRADER_RUNNER_N_RUNNERS") {
            s.runner.n_runners = n_runners.parse().map_err(|e| {
                Error::from(format!("Invalid n_runners value \"{n_runners}\": {e}"))
            })?;
        }
        if let Ok(interval) = std::env::var("AUTOGRADER_RUNNER_DATABASE_POLL_INTERVAL_SECONDS") {
            s.runner.database_poll_interval_seconds = interval.parse().map_err(|e| {
                Error::from(format!(
                    "Invalid database_poll_interval_seconds value \"{interval}\": {e}"
                ))
            })?;
        }
        if let Ok(podman_image) = std::env::var("AUTOGRADER_RUNNER_PODMAN_IMAGE") {
            s.runner.podman_image = podman_image;
        }
        if let Ok(podman_network_prefix) = std::env::var("AUTOGRADER_RUNNER_PODMAN_NETWORK_PREFIX")
        {
            s.runner.podman_network_prefix = podman_network_prefix;
        }
        if let Ok(mount_repo) = std::env::var("AUTOGRADER_RUNNER_MOUNT_REPO") {
            s.runner.mount_repo = mount_repo;
        }
        if let Ok(mount_tests) = std::env::var("AUTOGRADER_RUNNER_MOUNT_TESTS") {
            s.runner.mount_tests = mount_tests;
        }
        if let Ok(workspace_dir) = std::env::var("AUTOGRADER_RUNNER_WORKSPACE_DIR") {
            s.runner.workspace_dir = workspace_dir;
        }
        if let Ok(test_config) = std::env::var("AUTOGRADER_RUNNER_TEST_CONFIG") {
            s.runner.test_config = test_config;
        }
        //eprintln!("Done checking");

        Ok(s)
    }

    /// Sets up logging for the current process.
    pub fn setup_logging(self: &Self, prockind: &str) -> Result<(), Error> {
        use log::LevelFilter::{Debug, Info};

        std::fs::create_dir_all(&self.log.dir).map_err(|e| {
            eprintln!(
                "Error creating directory {} for the log file: {}",
                &self.log.dir, e
            );
            e.to_string()
        })?;

        let path_logfile = path_join(&self.log.dir, "log.out")?;
        let prockind = prockind.to_string();
        fern::Dispatch::new()
            .format(move |out, message, record| {
                out.finish(format_args!(
                    "[{0} {1} ({4}-{5}) {2}:{3}] {6}",
                    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"),
                    record.level(),
                    record.target(),
                    record.line().unwrap_or(0),
                    prockind,
                    std::process::id(),
                    message
                ))
            })
            .level(if self.log.verbose { Debug } else { Info })
            .chain(std::io::stderr())
            .chain(
                fern::log_file(&path_logfile).inspect_err(|e| {
                    eprintln!("Error setting up log file {}: {}", &path_logfile, e)
                })?,
            )
            .apply()?;
        Ok(())
    }
}
