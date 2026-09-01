//! Raw database row models, mapping table columns to fields in a struct.
//! Usually two structs per table, one for insertion (prefixed with New) and
//! another one for selecting.
//!
//! These should only be used for low-level database control. For everyday
//! interactions with the database, the aggregate models should be used
//! instead. See `db/models/source.rs` and `db/models/submission.rs`.

use chrono::{DateTime, Utc};
use diesel::prelude::{Identifiable, Insertable, Queryable, QueryableByName, Selectable};

#[derive(Debug, Clone, Queryable, Identifiable, QueryableByName, Selectable)]
#[diesel(table_name = crate::db::schema::submissions)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubmissionRow {
    pub id: i64,
    pub submitted_at: DateTime<Utc>,
    pub requested_tags: Vec<String>,
    pub origin_id: i64,
    pub report: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::db::schema::submissions)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewSubmissionRow {
    pub submitted_at: DateTime<Utc>,
    pub requested_tags: Vec<String>,
    pub origin_id: i64,
    pub report: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Queryable, Identifiable, QueryableByName, Selectable)]
#[diesel(table_name = crate::db::schema::submission_jobs)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubmissionJobRow {
    pub id: i64,
    pub submission_id: i64,
    pub tag: String,
    pub requested_as: Vec<String>,
    pub eligible_at: Option<DateTime<Utc>>,
    pub voided_at: Option<DateTime<Utc>>,
    pub assigned_runner_id: Option<i32>,
    pub status_code: i32,
    pub status_text: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::db::schema::submission_jobs)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewSubmissionJobRow {
    pub submission_id: i64,
    pub tag: String,
    pub requested_as: Vec<String>,
    pub eligible_at: Option<DateTime<Utc>>,
    pub voided_at: Option<DateTime<Utc>>,
    pub status_code: i32,
}

#[derive(Debug, Clone, Queryable, Identifiable, QueryableByName, Selectable)]
#[diesel(table_name = crate::db::schema::submission_origins)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubmissionOriginRow {
    pub id: i64,
    pub kind: i32,
    pub kind_id: i64,
    pub auth_key: String,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::db::schema::submission_origins)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewSubmissionOriginRow {
    pub kind: i32,
    pub kind_id: i64,
    pub auth_key: String,
}

#[derive(Debug, Clone, Queryable, Identifiable, QueryableByName, Selectable)]
#[diesel(table_name = crate::db::schema::submission_origin_github)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubmissionOriginGitHubRow {
    pub id: i64,
    pub domain: String,
    pub org: String,
    pub repo: String,
    pub ssh_url: String,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::db::schema::submission_origin_github)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewSubmissionOriginGitHubRow {
    pub domain: String,
    pub org: String,
    pub repo: String,
    pub ssh_url: String,
}

#[derive(Debug, Clone, Queryable, Identifiable, QueryableByName, Selectable)]
#[diesel(table_name = crate::db::schema::submission_info_github)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubmissionInfoGitHubRow {
    pub id: i64,
    pub submission_id: i64,
    pub github_origin_id: i64,
    pub user: String,
    pub commit: String,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::db::schema::submission_info_github)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewSubmissionInfoGitHubRow {
    pub submission_id: i64,
    pub github_origin_id: i64,
    pub user: String,
    pub commit: String,
}

#[derive(Debug, Clone, Queryable, Identifiable, QueryableByName, Selectable)]
#[diesel(table_name = crate::db::schema::submission_origin_gitlab)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubmissionOriginGitLabRow {
    pub id: i64,
    pub domain: String,
    pub namespace: String,
    pub repo: String,
    pub ssh_url: String,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::db::schema::submission_origin_gitlab)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewSubmissionOriginGitLabRow {
    pub domain: String,
    pub namespace: String,
    pub repo: String,
    pub ssh_url: String,
}

#[derive(Debug, Clone, Queryable, Identifiable, QueryableByName, Selectable)]
#[diesel(table_name = crate::db::schema::submission_info_gitlab)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubmissionInfoGitLabRow {
    pub id: i64,
    pub submission_id: i64,
    pub gitlab_origin_id: i64,
    pub user: String,
    pub commit: String,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::db::schema::submission_info_gitlab)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewSubmissionInfoGitLabRow {
    pub submission_id: i64,
    pub gitlab_origin_id: i64,
    pub user: String,
    pub commit: String,
}
