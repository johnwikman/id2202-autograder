use confique::Config;
use schemars::JsonSchema;
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

/// The autograder is configured through a TOML settings file passed via the
/// `-s` option on the entrypoint binary. Every setting listed below is required
/// — the autograder supplies no fallback values, so omitting any of them causes
/// startup to fail.
///
/// All relative paths are resolved relative to the directory containing the
/// settings file. Where an environment variable is listed, it takes precedence
/// over the value in the TOML file.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct Settings {
    /// Name to use when responding to requests, creating commits, etc.
    /// This does not have to correspond to a user name. Can be "ID2202
    /// Autograder", "Alice", or "Bob", etc.
    #[config(env = "AUTOGRADER_NAME")]
    pub name: String,

    /// Timeout (in seconds) for writing a file to the file system. Guards
    /// against a file system that stops making progress, such as an
    /// unresponsive network mount.
    #[config(env = "AUTOGRADER_FS_WRITE_TIMEOUT_SECONDS")]
    pub fs_write_timeout_seconds: u16,

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

    /// Relative dir to the configuration file. This is populated automatically
    /// on load and should not be explicitly specified. Not part of the settings
    /// file format, so it is kept out of the schema (and so out of the docs).
    #[config(default = "")]
    #[schemars(skip)]
    pub reldir: String,
}

/// Logging settings
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct LoggingSettings {
    /// Directory where to store log messages.
    #[config(env = "AUTOGRADER_LOG_DIR")]
    pub dir: String,

    /// Whether to output debug messages.
    #[config(env = "AUTOGRADER_LOG_VERBOSE", parse_env = parse_env_bool)]
    pub verbose: bool,
}

/// Settings for the entrypoint monitor loop
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct MonitorSettings {
    /// The frequency (in seconds) at which child processes are polled
    /// for whether they are alive or not.
    #[config(env = "AUTOGRADER_MONITOR_POLL_INTERVAL_SECONDS")]
    pub poll_interval_seconds: u16,
}

/// Settings for process notification
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct NotifySettings {
    /// Timeout (in milliseconds) for polling the notification file, to
    /// make sure that a process does not freeze due to polling.
    #[config(env = "AUTOGRADER_NOTIFY_POLL_TIMEOUT_MILLISEC")]
    pub poll_timeout_millisec: u16,
}

/// Settings for incoming submissions
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct SubmissionSettings {
    /// Maximum length of the concatenated tags that can be inserted
    /// into the database.
    #[config(env = "AUTOGRADER_SUBMISSION_MAX_TAG_LENGTH")]
    pub max_tag_length: usize,

    /// Maximum size of incoming JSON payload, in bytes.
    #[config(env = "AUTOGRADER_SUBMISSION_MAX_PAYLOAD")]
    pub max_payload: usize,

    /// A signature to place at the end of every comment made on GitLab.
    #[config(env = "AUTOGRADER_SUBMISSION_COMMENT_SIGNATURE")]
    pub comment_signature: String,

    /// Settings for submissions coming from a GitHub instance
    #[config(nested)]
    pub github: GitHubSettings,

    /// Settings for submissions coming from a GitLab instance
    #[config(nested)]
    pub gitlab: GitLabSettings,
}

/// Settings specific to incoming GitHub requests. See [ServerSettings] for
/// generic HTTP settings that applies to all incoming requests.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct GitHubSettings {
    /// Webhook secret used to validate incoming requests.
    #[config(env = "AUTOGRADER_SUBMISSION_GITHUB_WEBHOOK_SECRET")]
    pub webhook_secret: String,

    /// Information for specific instances.
    pub known_instances: Vec<GitHubServerSettings>,
}

/// Object format for each entry of `submission.github.known_instances`. These
/// per-instance settings cannot be provided through environment variables,
/// except that the `auth_token` of an already-defined instance may be overridden
/// via `AUTOGRADER_GITHUB_AUTH_TOKENS`, which holds semicolon-separated
/// `domain=token` pairs. See [GitHubSettings] for settings that apply to all
/// GitHub servers.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
#[schemars(title = "GitHub instance")]
pub struct GitHubServerSettings {
    /// The domain address at which the GitHub instance is hosted at.
    pub domain: String,

    /// The port at which the instance accepts SSH connections.
    pub ssh_port: u16,

    /// GitHub authorization token for using the API.
    pub auth_token: String,

    /// GitHub organizations to accept grading requests from.
    /// If not empty, the repository must be part of one of these organizations.
    pub allowed_orgs: Vec<String>,

    /// Allowed repository prefixes: if not empty, a repository
    /// must start with one of these prefix strings to be graded.
    pub allowed_repo_prefixes: Vec<String>,

    /// Allowed repository suffixes: if not empty, a repository
    /// must end with one of these suffix strings to be graded.
    pub allowed_repo_suffixes: Vec<String>,

    /// A repository is not allowed to start with any of these
    /// strings to be graded.
    pub prohibited_repo_prefixes: Vec<String>,

    /// A repository is not allowed to end with any of these
    /// strings to be graded.
    pub prohibited_repo_suffixes: Vec<String>,
}

/// Settings specific to incoming GitLab requests. See [ServerSettings] for
/// generic HTTP settings that applies to all incoming requests.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct GitLabSettings {
    /// Webhook secret used to validate incoming requests.
    #[config(env = "AUTOGRADER_SUBMISSION_GITLAB_WEBHOOK_SECRET")]
    pub webhook_secret: String,

    /// Information for specific instances.
    pub known_instances: Vec<GitLabServerSettings>,
}

/// Object format for each entry of `submission.gitlab.known_instances`. These
/// per-instance settings cannot be provided through environment variables,
/// except that the `auth_token` of an already-defined instance may be overridden
/// via `AUTOGRADER_GITLAB_AUTH_TOKENS`, which holds semicolon-separated
/// `domain=token` pairs. See [GitLabSettings] for settings that apply to all
/// GitLab servers.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
#[schemars(title = "GitLab instance")]
pub struct GitLabServerSettings {
    /// The domain address at which the GitLab instance is hosted at.
    pub domain: String,

    /// The port at which the instance accepts SSH connections.
    pub ssh_port: u16,

    /// GitLab authorization token for using the API.
    pub auth_token: String,

    /// GitLab namespaces to accept grading requests from. If
    /// not empty, the repository must be part of one of these namespaces.
    pub allowed_namespaces: Vec<String>,

    /// Allowed repository prefixes: if not empty, a repository
    /// must start with one of these prefix strings to be graded.
    pub allowed_repo_prefixes: Vec<String>,

    /// Allowed repository suffixes: if not empty, a repository
    /// must end with one of these suffix strings to be graded.
    pub allowed_repo_suffixes: Vec<String>,

    /// A repository is not allowed to start with any of these
    /// strings to be graded.
    pub prohibited_repo_prefixes: Vec<String>,

    /// A repository is not allowed to end with any of these
    /// strings to be graded.
    pub prohibited_repo_suffixes: Vec<String>,

    /// Whether or not HTTPS should be used when invoking the API. This
    /// should only ever be disabled when testing against a local GitLab
    /// instance. Use with caution.
    pub use_https: bool,
}

/// Connection details for the PostgreSQL database.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct PostgresSettings {
    /// Postgres username.
    #[config(env = "AUTOGRADER_POSTGRES_USER")]
    pub user: String,

    /// Password for the postgres user.
    #[config(env = "AUTOGRADER_POSTGRES_PASSWORD")]
    pub password: String,

    /// The hostname or IP address of the postgres database.
    #[config(env = "AUTOGRADER_POSTGRES_HOST")]
    pub host: String,

    /// The port used to connect to the postgres database (0–65535).
    #[config(env = "AUTOGRADER_POSTGRES_PORT")]
    pub port: u16,
}

/// Settings for the HTTP server that receives submissions and serves the API.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct ServerSettings {
    /// The IP address for the server binary to listen on.
    #[config(env = "AUTOGRADER_SERVER_ADDRESS")]
    pub address: String,

    /// The port that the server binary will listen on (0–65535).
    #[config(env = "AUTOGRADER_SERVER_PORT")]
    pub port: u16,

    /// Secrets used for client authentication
    #[config(nested)]
    pub secrets: ServerSecretsSettings,
}

/// Secrets used to authenticate clients of the REST API.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct ServerSecretsSettings {
    /// API auth tokens that can be used to fetch submission
    /// results over the REST API. Using the environment variable, multiple
    /// tokens can be specified using `;` separators, e.g. `TOKEN1;TOKEN2;TOKEN3`.
    #[config(env = "AUTOGRADER_SERVER_API_AUTH_TOKENS", parse_env = confique::env::parse::list_by_semicolon)]
    pub api_auth_tokens: Vec<String>,
}

/// Settings for runner processes
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct RunnerSettings {
    /// How many runners to spawn.
    #[config(env = "AUTOGRADER_RUNNER_N_RUNNERS")]
    pub n_runners: usize,

    /// How frequently (in seconds) a runner should poll the database,
    /// ignoring any notifications.
    #[config(env = "AUTOGRADER_RUNNER_DATABASE_POLL_INTERVAL_SECONDS")]
    pub database_poll_interval_seconds: u16,

    /// The docker/podman image to use for grading.
    #[config(env = "AUTOGRADER_RUNNER_PODMAN_IMAGE")]
    pub podman_image: String,

    /// The prefix to use for the network attached to the image. The
    /// network will be named as `{prefix}{runner_id}`.
    #[config(env = "AUTOGRADER_RUNNER_PODMAN_NETWORK_PREFIX")]
    pub podman_network_prefix: String,

    /// The directory inside the container where the repository will be
    /// mounted.
    #[config(env = "AUTOGRADER_RUNNER_MOUNT_REPO")]
    pub mount_repo: String,

    /// The directory inside the container where a test case will be
    /// located.
    #[config(env = "AUTOGRADER_RUNNER_MOUNT_TESTS")]
    pub mount_tests: String,

    /// Directory to use as a workspace, to store temporary files.
    #[config(env = "AUTOGRADER_RUNNER_WORKSPACE_DIR")]
    pub workspace_dir: String,

    /// Directory to store graded solutions in.
    #[config(env = "AUTOGRADER_RUNNER_SHADOW_DIR")]
    pub shadow_dir: String,

    /// Path to the root test configuration.
    #[config(env = "AUTOGRADER_RUNNER_TEST_CONFIG")]
    pub test_config: String,

    /// SSH keys to try when fetching a submitted repository, in order. The
    /// default SSH configuration is used when empty. **NOTE: an SSH server
    /// commonly refuses a connection if none of the first 6 keys worked.**
    #[config(env = "AUTOGRADER_RUNNER_SSH_KEYS", parse_env = confique::env::parse::list_by_semicolon)]
    pub ssh_keys: Vec<String>,

    /// Known hosts file to use when fetching a submitted repository. This can
    /// be populated by the `verify-ssh-hosts` entrypoint command.
    #[config(env = "AUTOGRADER_RUNNER_SSH_KNOWN_HOSTS")]
    pub ssh_known_hosts: String,
}

/// Settings controlling how grading results are reported.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct ReportingSettings {
    /// Markdown output settings
    #[config(nested)]
    pub markdown: ReportingMarkdownSettings,
}

/// Settings for markdown output on reports
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct ReportingMarkdownSettings {
    /// Symbol used to indicate the success of a test case, or a group
    /// of test cases.
    #[config(env = "AUTOGRADER_REPORTING_MD_SYMBOL_OK")]
    pub symbol_ok: String,

    /// Symbol used to indicate that a test or group of tests were not
    /// run.
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

    /// Whether to show an indicator on the top header of the
    /// submission results comment on GitHub, indicating whether all tags were
    /// successful or not.
    #[config(env = "AUTOGRADER_REPORTING_MD_SHOW_INDICATOR_SUBMISSION_HEADER", parse_env = parse_env_bool)]
    pub show_indicator_submission_header: bool,

    /// Whether to show an indicator for each individual tag-result
    /// header on the results comment on GitHub, indicating if this specific tag
    /// was successful or not.
    #[config(env = "AUTOGRADER_REPORTING_MD_SHOW_INDICATOR_TAG_HEADER", parse_env = parse_env_bool)]
    pub show_indicator_tag_header: bool,

    /// Truncate shown verbatim/code blocks that exceed this length.
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
        s.reldir = path_absolute_parent(path)?;

        //eprintln!("Converting relative paths to absolute paths");
        s.log.dir = path_absolute_join(&s.reldir, &s.log.dir)?;
        s.runner.workspace_dir = path_absolute_join(&s.reldir, &s.runner.workspace_dir)?;
        // SSH keys and known hosts are quoted directly into the SSH command as
        // a string, which cannot carry every byte that a path can.
        s.runner.ssh_known_hosts = path_absolute_join(&s.reldir, &s.runner.ssh_known_hosts)?;
        if shlex::try_quote(&s.runner.ssh_known_hosts).is_err() {
            return Err(Error::fs(
                "known hosts path cannot be quoted",
                &s.runner.ssh_known_hosts,
            ));
        }
        s.runner.ssh_keys = s
            .runner
            .ssh_keys
            .iter()
            .map(|k| {
                let key = path_absolute_join(&s.reldir, k)?;
                if shlex::try_quote(&key).is_err() {
                    return Err(Error::fs("SSH key path cannot be quoted", &key));
                }
                if !std::path::Path::new(&key).is_file() {
                    return Err(Error::fs("configured SSH key does not exist", &key));
                }
                Ok(key)
            })
            .collect::<Result<Vec<String>, Error>>()?;

        s.runner.shadow_dir = path_absolute_join(&s.reldir, &s.runner.shadow_dir)?;
        s.runner.test_config = path_absolute_join(&s.reldir, &s.runner.test_config)?;

        // Additional environment variables not captured by confique
        if let Ok(values) = std::env::var("AUTOGRADER_GITHUB_AUTH_TOKENS") {
            // Format: domain1=token;domain2=token
            for (domain, token) in values.split(";").filter_map(|p| p.split_once('=')) {
                match s
                    .submission
                    .github
                    .known_instances
                    .iter_mut()
                    .find(|gh| gh.domain == domain.trim())
                {
                    Some(gh_instance) => {
                        gh_instance.auth_token = token.trim().to_string();
                    }
                    None => {
                        log::warn!(
                            "Unrecognized domain in environment variable AUTOGRADER_GITHUB_AUTH_TOKENS"
                        );
                    }
                }
            }
        }

        if let Ok(values) = std::env::var("AUTOGRADER_GITLAB_AUTH_TOKENS") {
            for (domain, token) in values.split(";").filter_map(|p| p.split_once('=')) {
                match s
                    .submission
                    .gitlab
                    .known_instances
                    .iter_mut()
                    .find(|gl| gl.domain == domain.trim())
                {
                    Some(gl_instance) => {
                        gl_instance.auth_token = token.trim().to_string();
                    }
                    None => {
                        log::warn!(
                            "Unrecognized domain in environment variable AUTOGRADER_GITLAB_AUTH_TOKENS"
                        );
                    }
                }
            }
        }

        Ok(s)
    }

    /// Sets up logging for the current process.
    pub fn setup_logging(&self, prockind: &str) -> Result<(), Error> {
        use log::LevelFilter::{Debug, Info};

        std::fs::create_dir_all(&self.log.dir).map_err(|e| {
            eprintln!(
                "Error creating directory {} for the log file: {}",
                self.log.dir, e
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
