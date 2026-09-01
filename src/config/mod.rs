//! Modules related to configuration of the autograder.
pub mod settings;
pub mod tests;
pub mod utils;

pub use settings::{
    GitHubSettings, LoggingSettings, MonitorSettings, NotifySettings, PostgresSettings,
    ReportingMarkdownSettings, ReportingSettings, RunnerSettings, ServerSettings, Settings,
};

pub use tests::{
    group::{Test, TestConfig, TestGroup},
    tag::{tag_is_valid, tag_match, BuildConfig, Tag},
    Tests, TestsLoadingOptions,
};
