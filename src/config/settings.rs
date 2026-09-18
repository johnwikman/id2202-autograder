use std::borrow::Cow;
use std::collections::BTreeMap;

use confique::Config;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_inline_default::serde_inline_default;

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

/// The configured addresses of an instance, as they are written in the
/// settings file.
trait KnownInstanceSettings {
    /// The configured domain identifier of the instance.
    fn domain(&self) -> &str;
    /// Optional outbound hostname of the instance. If empty string, this
    /// should be interpreted to use the hostname from the configured domain.
    fn outbound(&self) -> &str;
}

/// Instances that can be addressed by a domain.
pub trait KnownInstance {
    /// The hostname of the domain, without the port number.
    fn host(&self) -> &str;

    /// The host that outbound traffic is sent to.
    fn outbound_host(&self) -> &str;

    /// An outbound domain to send requests to, which also includes the port
    /// number from the domain itself.
    fn outbound_domain(&self) -> Cow<'_, str>;
}

impl<T: KnownInstanceSettings> KnownInstance for T {
    fn host(&self) -> &str {
        match self.domain().rsplit_once(':') {
            Some((host, port)) if port.parse::<u16>().is_ok() => host,
            _ => self.domain(),
        }
    }

    fn outbound_host(&self) -> &str {
        match self.outbound() {
            "" => self.host(),
            host => host,
        }
    }

    fn outbound_domain(&self) -> Cow<'_, str> {
        match (self.outbound_host(), self.domain().rsplit_once(':')) {
            (host, Some((_, port))) if port.parse::<u16>().is_ok() => {
                Cow::Owned(format!("{host}:{port}"))
            }
            (host, _) => Cow::Borrowed(host),
        }
    }
}

/// The autograder is configured through a TOML settings file passed via the
/// `-s` option on the entrypoint binary. For example, to start the autograder
/// with the example settings:
///
/// ```sh
/// ./target/release/entrypoint -s example/settings.toml start
/// ```
///
/// Almost all values in the settings TOML can be overridden using environment
/// variables. If overridable, the environment variable is listed next to the
/// settings value. Example of overriding the `log.verbose` value:
///
/// ```sh
/// AUTOGRADER_LOG_VERBOSE=y ./target/release/entrypoint -s example/settings.toml start
/// ```
///
/// All settings that specify a path on the host system (i.e. not a path in a
/// podman grading container) will have their paths resolved relative to the
/// directory containing the settings file. This also includes values
/// overridden by environment variables.
///
/// # Warning
/// Every setting listed below is required with only a few exceptions. Failing
/// to provide them will cause the startup of the autograder to fail. A setting
/// that may be omitted will explicitly state its default behavior.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct Settings {
    /// Name to use when responding to requests, creating commits, etc.
    /// This does not have to correspond to a user name. Can be "ID2202
    /// Autograder", "Alice", or "Bob", etc.
    #[config(env = "AUTOGRADER_NAME")]
    pub name: String,

    #[config(nested)]
    pub log: LoggingSettings,

    #[config(nested)]
    pub monitor: MonitorSettings,

    #[config(nested)]
    pub timeout: TimeoutSettings,

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

    /// Absolute path of the configuration file itself, populated automatically
    /// on load just like `reldir`.
    #[config(default = "")]
    #[schemars(skip)]
    pub origin_path: String,
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

/// General settings for timeout related operations within the autograder
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct TimeoutSettings {
    /// Timeout (in milliseconds) for polling the notification file, to
    /// make sure that a process does not freeze due to polling.
    #[config(env = "AUTOGRADER_TIMEOUT_NOTIFY_POLL_MILLISEC")]
    pub notify_poll_millisec: u16,

    /// Timeout (in milliseconds) for sending HTTP requests. This ensures that
    /// a process does not freeze due to a slow endpoint.
    #[config(env = "AUTOGRADER_TIMEOUT_HTTP_SEND_MILLISEC")]
    pub http_send_millisec: u16,

    /// Timeout (in seconds) for writing a file to the file system. Guards
    /// against a file system that stops making progress, such as an
    /// unresponsive network mount.
    #[config(env = "AUTOGRADER_TIMEOUT_FS_WRITE_SECONDS")]
    pub fs_write_seconds: u16,
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

    /// Settings for direct submissions requests
    #[config(nested)]
    pub direct: DirectSettings,
}

/// Settings specific to incoming GitHub requests. See [ServerSettings] for
/// generic HTTP settings that applies to all incoming requests.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct GitHubSettings {
    /// Webhook secret used to validate incoming requests.
    #[config(env = "AUTOGRADER_SUBMISSION_GITHUB_WEBHOOK_SECRET")]
    pub webhook_secret: String,

    /// Information for specific instances.
    ///
    /// # Default
    /// No instances are configured if this is omitted.
    #[config(default = [])]
    pub known_instances: Vec<GitHubServerSettings>,
}

/// Object format for each entry of `submission.github.known_instances`. These
/// per-instance settings cannot be provided through environment variables,
/// except that the `auth_token` of an already-defined instance may be overridden
/// via `AUTOGRADER_GITHUB_AUTH_TOKENS`, which holds semicolon-separated
/// `domain=token` pairs. See [GitHubSettings] for settings that apply to all
/// GitHub servers.
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[schemars(title = "GitHub instance")]
pub struct GitHubServerSettings {
    /// The domain address at which the GitHub instance is hosted at.
    pub domain: String,

    /// The hostname of the server for outbound traffic. If empty, that means
    /// that the domain is also used for outbound traffic.
    pub outbound_host: String,

    /// The port at which the instance accepts SSH connections.
    pub ssh_port: u16,

    /// The user to use in the SSH connections. (Usually `git` for git cloning.)
    pub ssh_user: String,

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

impl KnownInstanceSettings for GitHubServerSettings {
    fn domain(&self) -> &str {
        &self.domain
    }
    fn outbound(&self) -> &str {
        &self.outbound_host
    }
}

/// Settings specific to incoming GitLab requests. See [ServerSettings] for
/// generic HTTP settings that applies to all incoming requests.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct GitLabSettings {
    /// Webhook secret used to validate incoming requests.
    #[config(env = "AUTOGRADER_SUBMISSION_GITLAB_WEBHOOK_SECRET")]
    pub webhook_secret: String,

    /// Information for specific instances.
    ///
    /// # Default
    /// No instances are configured if this is omitted.
    #[config(default = [])]
    pub known_instances: Vec<GitLabServerSettings>,
}

/// Object format for each entry of `submission.gitlab.known_instances`. These
/// per-instance settings cannot be provided through environment variables,
/// except that the `auth_token` of an already-defined instance may be overridden
/// via `AUTOGRADER_GITLAB_AUTH_TOKENS`, which holds semicolon-separated
/// `domain=token` pairs. See [GitLabSettings] for settings that apply to all
/// GitLab servers.
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[schemars(title = "GitLab instance")]
pub struct GitLabServerSettings {
    /// The domain address at which the GitLab instance is hosted at.
    pub domain: String,

    /// The hostname of the server for outbound traffic. If empty, that means
    /// that the domain is also used for outbound traffic.
    pub outbound_host: String,

    /// The port at which the instance accepts SSH connections.
    pub ssh_port: u16,

    /// The user to use in the SSH connections. (Usually `git` for git cloning.)
    pub ssh_user: String,

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

impl KnownInstanceSettings for GitLabServerSettings {
    fn domain(&self) -> &str {
        &self.domain
    }
    fn outbound(&self) -> &str {
        &self.outbound_host
    }
}

/// Settings specific to incoming direct submission requests.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct DirectSettings {
    /// Secret used to validate incoming submission requests.
    #[config(env = "AUTOGRADER_SUBMISSION_DIRECT_SECRET")]
    pub secret: String,

    /// Where incoming direct submissions should be stored on the local file
    /// system while waiting to be graded.
    #[config(env = "AUTOGRADER_SUBMISSION_DIRECT_STORAGE_DIR")]
    pub storage_dir: String,

    /// The maximum allowed size of an archive after it has been unpacked.
    #[config(env = "AUTOGRADER_SUBMISSION_DIRECT_MAX_UNPACKED_SIZE")]
    pub max_unpacked_size: usize,

    /// Known domains that are allowed to make direct submission requests.
    ///
    /// If specified using the environment variable, multiple keys are be
    /// separated by semicolons:
    ///
    /// ```sh
    /// AUTOGRADER_SUBMISSION_DIRECT_ALLOWED_DOMAINS="domain1;domain2;domain3"
    /// ```
    ///
    /// # Important
    /// The environment variable, if set, will discard any direct domains
    /// defined in the TOML file.
    #[config(env = "AUTOGRADER_SUBMISSION_DIRECT_ALLOWED_DOMAINS", parse_env = confique::env::parse::list_by_semicolon)]
    pub allowed_domains: Vec<String>,
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

    /// The port used to connect to the postgres database (`0-65535`).
    #[config(env = "AUTOGRADER_POSTGRES_PORT")]
    pub port: u16,
}

/// Settings for the HTTP server that receives submissions and serves the API.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct ServerSettings {
    /// The IP address for the server binary to listen on.
    #[config(env = "AUTOGRADER_SERVER_ADDRESS")]
    pub address: String,

    /// The port that the server binary will listen on (`0-65535`).
    #[config(env = "AUTOGRADER_SERVER_PORT")]
    pub port: u16,

    /// Secrets used for client authentication.
    #[config(nested)]
    pub secrets: ServerSecretsSettings,
}

/// Secrets used to authenticate clients of the REST API.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct ServerSecretsSettings {
    /// API auth tokens that can be used to fetch submission results over the
    /// REST API.
    ///
    /// If specified using the environment variable, multiple keys are be
    /// separated by semicolons:
    ///
    /// ```sh
    /// AUTOGRADER_SERVER_API_AUTH_TOKENS="token1;token2;token3"
    /// ```
    ///
    /// # Important
    /// The environment variable, if set, will discard any API tokens defined
    /// in the TOML file.
    #[config(env = "AUTOGRADER_SERVER_API_AUTH_TOKENS", parse_env = confique::env::parse::list_by_semicolon)]
    pub api_auth_tokens: Vec<String>,
}

/// Settings for runner processes.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct RunnerSettings {
    /// How many runners to spawn.
    #[config(env = "AUTOGRADER_RUNNER_N_RUNNERS")]
    pub n_runners: usize,

    /// How frequently (in seconds) a runner should poll the database,
    /// ignoring any notifications.
    #[config(env = "AUTOGRADER_RUNNER_DATABASE_POLL_INTERVAL_SECONDS")]
    pub database_poll_interval_seconds: u16,

    /// Directory to use as a workspace, to store temporary files.
    #[config(env = "AUTOGRADER_RUNNER_WORKSPACE_DIR")]
    pub workspace_dir: String,

    /// Directory to store graded solutions in.
    #[config(env = "AUTOGRADER_RUNNER_SHADOW_DIR")]
    pub shadow_dir: String,

    /// Path to the root test configuration.
    #[config(env = "AUTOGRADER_RUNNER_TEST_CONFIG")]
    pub test_config: String,

    /// Paths to SSH keys to try when fetching a submitted repository, in
    /// order. The default SSH configuration is used when empty.
    ///
    /// If specified using the environment variable, multiple keys are be
    /// separated by semicolons:
    ///
    /// ```sh
    /// AUTOGRADER_RUNNER_SSH_KEYS="path_to_key1;path_to_key2;path_to_key3"
    /// ```
    ///
    /// # Important
    /// The environment variable, if set, will discard any SSH key paths
    /// defined in the TOML file.
    ///
    /// # Note
    /// An SSH server commonly refuses a connection if none of the first 6 keys worked.
    #[config(env = "AUTOGRADER_RUNNER_SSH_KEYS", parse_env = confique::env::parse::list_by_semicolon)]
    pub ssh_keys: Vec<String>,

    /// Path to known hosts file to use when fetching a submitted repository.
    /// This can be populated by the `verify-ssh-hosts` entrypoint command.
    #[config(env = "AUTOGRADER_RUNNER_SSH_KNOWN_HOSTS")]
    pub ssh_known_hosts: String,

    /// Settings related to podman images, containers, and networks.
    #[config(nested)]
    pub podman: PodmanSettings,
}

/// Settings specific for handling podman images.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct PodmanSettings {
    /// The prefix to use for the network attached to the image. The
    /// network will be named as `{prefix}{runner_id}`.
    #[config(env = "AUTOGRADER_RUNNER_PODMAN_NETWORK_PREFIX")]
    pub network_prefix: String,

    /// Declarations of the available images to be used by the test specification.
    ///
    /// # Default
    /// No images are configured if omitted. However, omitting this will break
    /// the test configuration which requires at least one image to function.
    #[config(default = {})]
    pub images: BTreeMap<String, PodmanImageSettings>,
}

/// Declaration and specification for specific podman images.
#[serde_inline_default]
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[schemars(title = "Podman image")]
pub struct PodmanImageSettings {
    /// The name of the image, formatted as `{repo}:{tag}`.
    pub image: String,

    /// Optional information about how to build this image if it does not
    /// exist.
    ///
    /// # Default
    /// If not specified, then the image is assumed to be fetchable using
    /// `podman pull`.
    pub build: Option<PodmanImageBuildSettings>,

    /// A directory inside the container that can be used as a tmp directory
    /// during grading.
    ///
    /// # Default
    /// `/tmp` if not provided.
    #[serde_inline_default("/tmp".to_string())]
    pub tmpdir: String,

    /// Path inside the container where the code to run shall be mounted.
    ///
    /// # Default
    /// `/mnt/code` if not provided.
    #[serde_inline_default("/mnt/code".to_string())]
    pub mount_code: String,

    /// Path inside the container where the tests that are going to be run
    /// shall be mounted.
    ///
    /// # Default
    /// `/mnt/tests` if not provided.
    #[serde_inline_default("/mnt/tests".to_string())]
    pub mount_tests: String,
}

/// Information about how to build a podman image.
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[schemars(title = "Podman build")]
pub struct PodmanImageBuildSettings {
    /// The path (or context) that the image should be built in.
    pub path: String,

    /// The Containerfile/Dockerfile to use when building the image, specified
    /// relative to the provided `path`.
    pub file: String,
}

/// Settings controlling how grading results are reported.
#[derive(Config, Deserialize, JsonSchema, Debug, Clone)]
pub struct ReportingSettings {
    /// Maximum number of failed test cases revealed to the student in a single
    /// submission.
    #[config(env = "AUTOGRADER_REPORTING_SHOWN_FAILURES")]
    pub shown_failures: usize,

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

    /// Symbol used to indicate that a tag/job is waiting to be graded.
    #[config(env = "AUTOGRADER_REPORTING_MD_SYMBOL_WAITING")]
    pub symbol_waiting: String,

    /// Symbol used to indicate that a tag was voided. I.e. it will not be
    /// graded.
    #[config(env = "AUTOGRADER_REPORTING_MD_SYMBOL_VOIDED")]
    pub symbol_voided: String,

    /// Whether to show an indicator on the top header of the submission
    /// results comment on GitHub, indicating whether all tags were successful
    /// or not.
    #[config(env = "AUTOGRADER_REPORTING_MD_SHOW_INDICATOR_SUBMISSION_HEADER", parse_env = parse_env_bool)]
    pub show_indicator_submission_header: bool,

    /// Whether to show an indicator for each individual tag-result header on
    /// the results comment on GitHub, indicating if this specific tag was
    /// successful or not.
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
        s.origin_path = match std::path::Path::new(path).file_name() {
            Some(name) => path_absolute_join(&s.reldir, name)?,
            None => return Err(Error::fs("settings path names no file", path)),
        };

        //eprintln!("Converting relative paths to absolute paths");
        s.log.dir = path_absolute_join(&s.reldir, &s.log.dir)?;
        s.submission.direct.storage_dir =
            path_absolute_join(&s.reldir, &s.submission.direct.storage_dir)?;
        s.runner.workspace_dir = path_absolute_join(&s.reldir, &s.runner.workspace_dir)?;
        // SSH keys and known hosts are quoted directly into the SSH command as
        // a string, which cannot carry every byte that a path can.
        s.runner.ssh_known_hosts = path_absolute_join(&s.reldir, &s.runner.ssh_known_hosts)?;
        if shlex::try_quote(&s.runner.ssh_known_hosts).is_err() {
            return Err(Error::fs("known hosts path cannot be quoted", &s.runner.ssh_known_hosts));
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
        for img in s.runner.podman.images.values_mut() {
            if let Some(build) = img.build.as_mut() {
                build.path = path_absolute_join(&s.reldir, &build.path)?;
                build.file = path_absolute_join(&build.path, &build.file)?;
            }
        }

        /// Helper function for parsing variables provided a semicolon
        /// separated associations for preconfigured domains. I.e.
        ///
        /// ```txt
        /// <domain1>=<value>;<domain2>=<value>;...
        /// ```
        fn env_domain_pair_set<T: KnownInstanceSettings>(
            key: &str,
            instances: &mut [T],
            mut set: impl FnMut(&mut T, String),
        ) {
            let Ok(values) = std::env::var(key) else {
                return;
            };
            for (domain, value) in values.split(';').filter_map(|p| p.split_once('=')) {
                match instances.iter_mut().find(|i| i.domain() == domain.trim()) {
                    Some(instance) => set(instance, value.trim().to_string()),
                    None => {
                        log::warn!("Unrecognized domain {domain} in environment variable {key}")
                    }
                }
            }
        }

        let gh = &mut s.submission.github.known_instances;
        env_domain_pair_set("AUTOGRADER_GITHUB_AUTH_TOKENS", gh, |gh, v| gh.auth_token = v);
        env_domain_pair_set("AUTOGRADER_GITHUB_OUTBOUND_HOSTS", gh, |gh, v| gh.outbound_host = v);

        let gl = &mut s.submission.gitlab.known_instances;
        env_domain_pair_set("AUTOGRADER_GITLAB_AUTH_TOKENS", gl, |gl, v| gl.auth_token = v);
        env_domain_pair_set("AUTOGRADER_GITLAB_OUTBOUND_HOSTS", gl, |gl, v| gl.outbound_host = v);

        Ok(s)
    }

    /// Sets up logging for the current process.
    pub fn setup_logging(&self, prockind: &str) -> Result<(), Error> {
        use log::LevelFilter::{Debug, Info};

        std::fs::create_dir_all(&self.log.dir).map_err(|e| {
            eprintln!("Error creating directory {} for the log file: {}", self.log.dir, e);
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
