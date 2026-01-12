use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::ToPrimitive;

use diesel::prelude::{
    Associations, Identifiable, Insertable, Queryable, QueryableByName, Selectable,
};
use std::time::SystemTime;

/// Cheekily using HTTP-like codes here, although they have nothing to do with
/// the HTTP protocol.
#[derive(Debug, Clone, Copy, FromPrimitive, ToPrimitive)]
pub enum SubmissionStatusCode {
    NotStarted = 0,
    Running = 100,
    Success = 200,
    SubmissionError = 400,
    BuildError = 401,
    BuildTimedOut = 402,
    TestCasesFailed = 403,
    TestCasesTimedOut = 404,
    SubmissionTimedOut = 405,
    OutputLimitExceeded = 406,
    // Internal failure by the autograder
    AutograderFailure = 500,
}

impl SubmissionStatusCode {
    /// Whether this status code indicates that a submission has finished
    /// running.
    pub fn is_finished(&self) -> bool {
        (*self as i32) >= 200
    }

    /// Whether or not this status code indicates that an error has occurred
    pub fn is_error(&self) -> bool {
        (*self as i32).to_i32().unwrap() >= 400
    }
}

impl std::fmt::Display for SubmissionStatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotStarted => write!(f, "Not Started"),
            Self::Running => write!(f, "Running"),
            Self::Success => write!(f, "Success"),
            Self::SubmissionError => write!(f, "Submission Error"),
            Self::BuildError => write!(f, "Build Error"),
            Self::BuildTimedOut => write!(f, "Build Timed Out"),
            Self::TestCasesFailed => write!(f, "Test Cases Failed"),
            Self::TestCasesTimedOut => write!(f, "Test Cases Timed Out"),
            Self::SubmissionTimedOut => write!(f, "Submission Timed Out"),
            Self::OutputLimitExceeded => write!(f, "Output Limit Exceeded"),
            Self::AutograderFailure => write!(f, "Autograder Failure"),
        }
    }
}

/// Submission Source Kind
#[derive(Debug, Clone, Copy, FromPrimitive, ToPrimitive)]
pub enum SubmissionSourceKind {
    GitHub = 0,
}

impl std::fmt::Display for SubmissionSourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitHub => write!(f, "GitHub"),
        }
    }
}

#[derive(Debug, Clone, Queryable, Identifiable, QueryableByName, Selectable, Associations)]
#[diesel(belongs_to(SubmissionSource, foreign_key = source_id))]
#[diesel(table_name = crate::db::schema::submissions)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Submission {
    pub id: i64,
    pub date_submitted: SystemTime,
    pub assigned_runner_id: Option<i32>,
    pub grading_tags: String,
    pub exec_finished: bool,
    pub exec_status_code: i32,
    pub exec_status_text: Option<String>,
    pub exec_date_started: Option<SystemTime>,
    pub exec_date_finished: Option<SystemTime>,
    pub exec_report: Option<serde_json::Value>,
    pub source_id: i64,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::db::schema::submissions)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewSubmission {
    pub date_submitted: SystemTime,
    pub grading_tags: String,
    pub exec_finished: bool,
    pub exec_status_code: i32,
    pub source_id: i64,
}

#[derive(Debug, Clone, Queryable, Identifiable, QueryableByName, Selectable)]
#[diesel(table_name = crate::db::schema::submission_sources)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubmissionSource {
    pub id: i64,
    pub kind: i32,
    pub kind_id: i64,
    pub auth_key: String,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::db::schema::submission_sources)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewSubmissionSource {
    pub kind: i32,
    pub kind_id: i64,
    pub auth_key: String,
}

#[derive(Debug, Clone, Queryable, Identifiable, QueryableByName, Selectable)]
#[diesel(table_name = crate::db::schema::submission_source_github)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubmissionSourceGitHub {
    pub id: i64,
    pub domain: String,
    pub org: String,
    pub repo: String,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::db::schema::submission_source_github)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewSubmissionSourceGitHub {
    pub domain: String,
    pub org: String,
    pub repo: String,
}

#[derive(Debug, Clone, Queryable, Identifiable, QueryableByName, Selectable, Associations)]
#[diesel(belongs_to(Submission))]
#[diesel(belongs_to(SubmissionSourceGitHub, foreign_key = github_source_id))]
#[diesel(table_name = crate::db::schema::submission_info_github)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubmissionInfoGitHub {
    pub id: i64,
    pub submission_id: i64,
    pub github_source_id: i64,
    pub user: String,
    pub commit: String,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::db::schema::submission_info_github)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewSubmissionInfoGitHub {
    pub submission_id: i64,
    pub github_source_id: i64,
    pub user: String,
    pub commit: String,
}

/// Enumerator containing information over the possible sources
#[derive(Debug, Clone)]
pub enum SubmissionInfo {
    GitHub {
        sub: Submission,
        src: SubmissionSource,
        gh_src: SubmissionSourceGitHub,
        gh_info: SubmissionInfoGitHub,
    },
}

impl SubmissionInfo {
    pub fn get_submission(&self) -> &Submission {
        match self {
            Self::GitHub { sub, .. } => sub,
        }
    }
    pub fn get_source(&self) -> &SubmissionSource {
        match self {
            Self::GitHub { src, .. } => src,
        }
    }
}
