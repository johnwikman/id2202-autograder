use confique::Config;
use serde::Deserialize;

use crate::error::Error;
use crate::utils::{path_absolute_join, path_absolute_parent, path_join};

/// Parses an environment variable value into a boolean. Using a custom parse
/// function here to allow more flexibility in allowed values.
fn parse_env_bool(s: &str) -> Result<bool, Error> {
    match s.to_lowercase().as_str() {
        "true" | "t" | "yes" | "y" => Ok(true),
        "false" | "f" | "no" | "n" => Ok(false),
        _ => Error::err_parse_type("bool", s),
    }
}

/// Program-wide settings
#[derive(Config, Deserialize, Debug, Clone)]
pub struct Settings {
    /// Name to use when responding to requests, creating commits, etc. This
    /// does not have to correspond to a user name. Can be "ID2202 Autograder",
    /// "Alice", or "Bob", etc.
    pub name: String,

    #[config(nested)]
    pub log: LoggingSettings,

    #[config(nested)]
    pub monitor: MonitorSettings,

    #[config(nested)]
    pub notify: NotifySettings,

    #[config(nested)]
    pub submission: SubmissionSettings,

    #[config(nested)]
    pub postgres: PostgresSettings,

    #[config(nested)]
    pub server: ServerSettings,

    #[config(nested)]
    pub runner: RunnerSettings,

    #[config(nested)]
    pub reporting: ReportingSettings,

    /// Relative dir to the configuration file
    #[config(default = "")]
    pub reldir: String,
}

/// Logging settings
#[derive(Config, Deserialize, Debug, Clone)]
pub struct LoggingSettings {
    /// Directory where to store log messages
    #[config(env = "AUTOGRADER_LOG_DIR")]
    pub dir: String,

    /// Whether to output debug messages.
    #[config(env = "AUTOGRADER_LOG_VERBOSE", parse_env = parse_env_bool)]
    pub verbose: bool,
}

/// Settings for the entrypoint monitor loop
#[derive(Config, Deserialize, Debug, Clone)]
pub struct MonitorSettings {
    /// The frequency at which child processes are polled for whether they are
    /// alive or not.
    pub poll_interval_seconds: u16,
}

/// Settings for process notification
#[derive(Config, Deserialize, Debug, Clone)]
pub struct NotifySettings {
    /// Timeout for polling the notification file, to make sure that a process
    /// does not freeze due to polling.
    pub poll_timeout_millisec: u16,
}

/// Settings for incoming submissions
#[derive(Config, Deserialize, Debug, Clone)]
pub struct SubmissionSettings {
    /// Maximum length of the concatenated tags that can be inserted into the
    /// database.
    #[config(env = "AUTOGRADER_SUBMISSION_MAX_TAG_LENGTH")]
    pub max_tag_length: usize,

    /// Maximum size of incoming JSON payload
    #[config(env = "AUTOGRADER_SUBMISSION_MAX_PAYLOAD")]
    pub max_payload: usize,

    /// A signature to place at the end of every comment made on GitLab
    #[config(env = "AUTOGRADER_SUBMISSION_COMMENT_SIGNATURE")]
    pub comment_signature: String,

    /// Settings for submissions coming from a GitHub instance
    #[config(nested)]
    pub github: GitHubSettings,

    /// Settings for submissions coming from a GitLab instance
    #[config(nested)]
    pub gitlab: GitLabSettings,
}

/// Settings specific to incoming GitHub requests. See `ServerSettings` for
/// generic HTTP settings that applies to all incoming requests.
#[derive(Config, Deserialize, Debug, Clone)]
pub struct GitHubSettings {
    /// Webhook secret used to validate incoming requests
    #[config(env = "AUTOGRADER_SUBMISSION_GITHUB_WEBHOOK_SECRET")]
    pub webhook_secret: String,

    /// Information for specific instances.
    pub known_instances: Vec<GitHubServerSettings>,
}

/// Settings for a single GitHub server. See `GitHubSettings` for settings that
/// apply to all GitHub servers.
#[derive(Config, Deserialize, Debug, Clone)]
pub struct GitHubServerSettings {
    /// The domain address at which the GitHub instance is hosted at
    pub domain: String,

    /// GitHub authorization token for using the API
    pub auth_token: String,

    /// GitHub organization to accept grading requests from. If not empty, the repository must be part of one of these organizations.
    pub allowed_orgs: Vec<String>,

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
}

/// Settings specific to incoming GitLab requests. See `ServerSettings` for
/// generic HTTP settings that applies to all incoming requests.
#[derive(Config, Deserialize, Debug, Clone)]
pub struct GitLabSettings {
    /// Webhook secret used to validate incoming requests
    #[config(env = "AUTOGRADER_SUBMISSION_GITLAB_WEBHOOK_SECRET")]
    pub webhook_secret: String,

    /// Information for specific instances.
    pub known_instances: Vec<GitLabServerSettings>,
}

/// Settings for a single GitLab server. See `GitLabSettings` for settings that
/// apply to all GitLab servers.
#[derive(Config, Deserialize, Debug, Clone)]
pub struct GitLabServerSettings {
    /// The domain address at which the GitLab instance is hosted at
    pub domain: String,

    /// GitLab authorization token for using the API
    pub auth_token: String,

    /// GitLab namespaces to accept grading requests from. If not empty, the
    /// repository must be part of one of these namespaces.
    pub allowed_namespaces: Vec<String>,

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

    /// Whether or not HTTPS should be used when invoking the API
    ///
    /// ### Warning
    /// This should only ever be used when testing against a local GitLab
    /// instance. Use with caution.
    pub use_https: bool,
}

#[derive(Config, Deserialize, Debug, Clone)]
pub struct PostgresSettings {
    /// Postgres username
    #[config(env = "AUTOGRADER_POSTGRES_USER")]
    pub user: String,

    /// Password for the postgres user
    #[config(env = "AUTOGRADER_POSTGRES_PASSWORD")]
    pub password: String,

    /// The hostname or IP address of the postgres database
    #[config(env = "AUTOGRADER_POSTGRES_HOST")]
    pub host: String,

    /// The port used to connect to the postgres database
    #[config(env = "AUTOGRADER_POSTGRES_PORT")]
    pub port: u16,
}

#[derive(Config, Deserialize, Debug, Clone)]
pub struct ServerSettings {
    /// The IP address for the server binary to listen on
    #[config(env = "AUTOGRADER_SERVER_ADDRESS")]
    pub address: String,

    /// The port that the server binary will to listen on
    #[config(env = "AUTOGRADER_SERVER_PORT")]
    pub port: u16,

    /// Secrets used for client authentication
    #[config(nested)]
    pub secrets: ServerSecretsSettings,
}

#[derive(Config, Deserialize, Debug, Clone)]
pub struct ServerSecretsSettings {
    /// API auth tokens that can be used to fetch submission results over the
    /// REST API. Using the environment variable, multiple tokens can be
    /// specified using `;` separators, e.g. `TOKEN1;TOKEN2;TOKEN3`, etc.
    #[config(env = "AUTOGRADER_SERVER_API_AUTH_TOKENS", parse_env = confique::env::parse::list_by_semicolon)]
    pub api_auth_tokens: Vec<String>,
}

/// Settings for runner processes
#[derive(Config, Deserialize, Debug, Clone)]
pub struct RunnerSettings {
    /// How many runners to spawn
    #[config(env = "AUTOGRADER_RUNNER_N_RUNNERS")]
    pub n_runners: usize,

    /// How frequently that a runner should poll the database, ignoring any
    /// notifications
    #[config(env = "AUTOGRADER_RUNNER_DATABASE_POLL_INTERVAL_SECONDS")]
    pub database_poll_interval_seconds: u16,

    /// The docker/podman image to use for grading
    #[config(env = "AUTOGRADER_RUNNER_PODMAN_IMAGE")]
    pub podman_image: String,

    /// The prefix to use for the network attached to the image. The network
    /// will be named as "{prefix}{runner_id}"
    #[config(env = "AUTOGRADER_RUNNER_PODMAN_NETWORK_PREFIX")]
    pub podman_network_prefix: String,

    /// The directory inside the container where the repository will be mounted
    #[config(env = "AUTOGRADER_RUNNER_MOUNT_REPO")]
    pub mount_repo: String,

    /// The directory inside the container where a test case will be located
    #[config(env = "AUTOGRADER_RUNNER_MOUNT_TESTS")]
    pub mount_tests: String,

    /// Directory to use as a workspace, to store temporary files
    #[config(env = "AUTOGRADER_RUNNER_WORKSPACE_DIR")]
    pub workspace_dir: String,

    /// Directory to store graded solutions in
    #[config(env = "AUTOGRADER_RUNNER_SHADOW_DIR")]
    pub shadow_dir: String,

    /// Path to the test configuration
    #[config(env = "AUTOGRADER_RUNNER_TEST_CONFIG")]
    pub test_config: String,
}

#[derive(Config, Deserialize, Debug, Clone)]
pub struct ReportingSettings {
    /// Markdown output settings
    #[config(nested)]
    pub markdown: ReportingMarkdownSettings,
}

/// Settings for markdown output on reports
#[derive(Config, Deserialize, Debug, Clone)]
pub struct ReportingMarkdownSettings {
    /// Symbol used to indicate the success of a test case, or a group of test
    /// cases.
    #[config(env = "AUTOGRADER_REPORTING_MD_SYMBOL_OK")]
    pub symbol_ok: String,

    /// Symbol used to indicate that a test or group of tests were not run.
    #[config(env = "AUTOGRADER_REPORTING_MD_SYMBOL_SKIPPED")]
    pub symbol_skipped: String,

    /// Symbol used to indicate that a test or group of tests failed.
    #[config(env = "AUTOGRADER_REPORTING_MD_SYMBOL_FAILED")]
    pub symbol_failed: String,

    /// Celebratory symbol used when a tag has successfully passed.
    #[config(env = "AUTOGRADER_REPORTING_MD_SYMBOL_TAGSUCCESS")]
    pub symbol_tagsuccess: String,

    /// Symbol used to represent a build stage.
    #[config(env = "AUTOGRADER_REPORTING_MD_SYMBOL_BUILD")]
    pub symbol_build: String,

    /// Whether to show an indicator on the top header of the submission
    /// results comment on GitHub, indicating whether all tags where successful
    /// or not.
    #[config(env = "AUTOGRADER_REPORTING_MD_SHOW_INDICATOR_SUBMISSION_HEADER", parse_env = parse_env_bool)]
    pub show_indicator_submission_header: bool,

    /// Whether to show an indicator for each individual tag-result header on
    /// the results comment on GitHub, indicating if this specific tag was
    /// successful or not.
    #[config(env = "AUTOGRADER_REPORTING_MD_SHOW_INDICATOR_TAG_HEADER", parse_env = parse_env_bool)]
    pub show_indicator_tag_header: bool,

    /// Truncate shown verbatim/code blocks that exceeds this length.
    #[config(env = "AUTOGRADER_REPORTING_MD_TRUNCATE_LEN")]
    pub truncate_len: usize,
}

impl Settings {
    /// Loads settings from the specified path
    pub fn load(path: &str) -> Result<Self, Error> {
        let mut s: Settings = Config::builder()
            .env()
            .file(path)
            .load()
            .inspect_err(|e| eprintln!("Could not load settings from \"{path}\": {e}"))
            .map_err(|e| Error::load_config(path).with_cause(Box::new(e)))?;

        //eprintln!("Setting up canonical dir that the settings file is located in");
        s.reldir = path_absolute_parent(&path)?;

        //eprintln!("Converting relative paths to absolute paths");
        s.log.dir = path_absolute_join(&s.reldir, &s.log.dir)?;
        s.runner.workspace_dir = path_absolute_join(&s.reldir, &s.runner.workspace_dir)?;
        s.runner.shadow_dir = path_absolute_join(&s.reldir, &s.runner.shadow_dir)?;
        s.runner.test_config = path_absolute_join(&s.reldir, &s.runner.test_config)?;

        // Additional environment variables not captured by confique
        if let Ok(values) = std::env::var("AUTOGRADER_GITHUB_AUTH_TOKENS") {
            // Format: domain1=token;domain2=token
            for v in values.split(";") {
                match v.split("=").collect::<Vec<_>>().as_slice() {
                    &[domain, token] => {
                        match s
                            .submission
                            .github
                            .known_instances
                            .iter_mut()
                            .find(|gh| gh.domain == domain.trim())
                        {
                            Some(gh_instance) => {
                                gh_instance.auth_token = token.to_string();
                            }
                            None => {
                                log::warn!(
                                    "Unrecognized domain in environment variable AUTOGRADER_GITHUB_AUTH_TOKENS"
                                );
                            }
                        }
                    }
                    _ => {
                        log::warn!(
                            "Invalid format for environment variable AUTOGRADER_GITHUB_AUTH_TOKENS"
                        );
                    }
                }
            }
        }

        if let Ok(values) = std::env::var("AUTOGRADER_GITLAB_AUTH_TOKENS") {
            for v in values.split(";") {
                match v.split("=").collect::<Vec<_>>().as_slice() {
                    &[domain, token] => {
                        match s
                            .submission
                            .gitlab
                            .known_instances
                            .iter_mut()
                            .find(|gl| gl.domain == domain.trim())
                        {
                            Some(gl_instance) => {
                                gl_instance.auth_token = token.to_string();
                            }
                            None => {
                                log::warn!(
                                    "Unrecognized domain in environment variable AUTOGRADER_GITLAB_AUTH_TOKENS"
                                );
                            }
                        }
                    }
                    _ => {
                        log::warn!(
                            "Invalid format for environment variable AUTOGRADER_GITLAB_AUTH_TOKENS"
                        );
                    }
                }
            }
        }

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
            Error::fs("error creating log file directory", &self.log.dir).with_cause(Box::new(e))
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
            .chain(fern::log_file(&path_logfile).map_err(|e| {
                Error::fs("setting up log file", &path_logfile).with_cause(Box::new(e))
            })?)
            .apply()
            .map_err(|e| e.into())
    }
}
