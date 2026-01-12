/// Modules related to configuration of the autograder.
pub mod settings;
pub mod tests;

pub use settings::{
    GitHubSettings, LoggingSettings, MonitorSettings, NotifySettings, PostgresSettings,
    ReportingMarkdownSettings, ReportingSettings, RunnerSettings, ServerSettings, Settings,
};

pub use tests::{
    tag_is_valid, tag_match, Tag, TagBuildConfig, Test, TestDefault, TestGroup, Testkind, Tests,
};
