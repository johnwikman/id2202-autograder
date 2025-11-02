use num_derive::{FromPrimitive, ToPrimitive};

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
    // Internal failure by the autograder
    AutograderFailure = 500,
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
            Self::AutograderFailure => write!(f, "Autograder Failure"),
        }
    }
}

use diesel::prelude::{Insertable, Queryable, QueryableByName, Selectable};
use std::time::SystemTime;

#[derive(Debug, Clone, Queryable, QueryableByName, Selectable)]
#[diesel(table_name = crate::db::schema::submissions)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Submission {
    pub id: i64,
    pub date_submitted: SystemTime,
    pub assigned_runner: Option<i32>,
    pub grading_tags: String,
    pub exec_finished: bool,
    pub exec_status_code: i32,
    pub exec_status_text: Option<String>,
    pub exec_date_started: Option<SystemTime>,
    pub exec_date_finished: Option<SystemTime>,
    pub github_address: String,
    pub github_org: String,
    pub github_repo: String,
    pub github_user: String,
    pub github_commit: String,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::db::schema::submissions)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewGitHubSubmission {
    pub date_submitted: SystemTime,
    pub grading_tags: String,
    pub exec_finished: bool,
    pub exec_status_code: i32,
    pub github_address: String,
    pub github_org: String,
    pub github_repo: String,
    pub github_user: String,
    pub github_commit: String,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::db::schema::runners)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Runner {
    pub id: i32,
    pub pid: Option<i64>,
    pub last_pinged: Option<SystemTime>,
}
