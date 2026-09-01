use std::collections::BTreeMap;

use actix_web::{
    http::{header, StatusCode},
    HttpRequest, HttpResponse,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Serialize;
use utoipa::ToSchema;

use derive_more::derive::{Display, Error};

use id2202_autograder::{
    config::{BuildConfig, Tag, Tests},
    db::models::{
        origin::StoredOriginEnum, JobStatus, Submission, SubmissionJobPlain,
        SubmissionJobWithReport, SubmissionOrigin, SubmissionWithReports,
    },
    reporting::Report,
};

macro_rules! schema_callback {
    ($struct_ident:path) => {
        || async {
            use actix_web::{http::StatusCode, HttpResponse};
            let schema = schemars::schema_for!($struct_ident);
            HttpResponse::build(StatusCode::OK).json(schema)
        }
    };
}
pub(crate) use schema_callback;

/// Problem details, as defined by [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457).
#[derive(Serialize, Debug, Display, Error, JsonSchema, ToSchema)]
#[display("error response: {status} on {instance}")]
pub struct ErrorResponse {
    /// HTTP status code.
    status: u16,
    /// Short summary of the problem type, e.g. `"Not Found"`.
    title: String,
    /// Human-readable detail.
    detail: String,
    /// Request path.
    instance: String,
    /// Request method.
    method: String,
}

impl actix_web::error::ResponseError for ErrorResponse {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).content_type("application/problem+json").json(self)
    }
    fn status_code(&self) -> actix_web::http::StatusCode {
        StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl ErrorResponse {
    fn new(req: &HttpRequest, status: StatusCode, detail: &str) -> ErrorResponse {
        ErrorResponse {
            status: status.as_u16(),
            title: status.canonical_reason().unwrap_or_default().to_string(),
            detail: detail.to_string(),
            instance: req.path().to_string(),
            method: req.method().to_string(),
        }
    }
    pub fn unauthorized(req: &HttpRequest, msg: &str) -> ErrorResponse {
        Self::new(req, StatusCode::UNAUTHORIZED, msg)
    }
    pub fn bad_request(req: &HttpRequest, msg: &str) -> ErrorResponse {
        Self::new(req, StatusCode::BAD_REQUEST, msg)
    }
    pub fn not_found(req: &HttpRequest, msg: &str) -> ErrorResponse {
        Self::new(req, StatusCode::NOT_FOUND, msg)
    }
    pub fn internal_server_error(req: &HttpRequest) -> ErrorResponse {
        Self::new(req, StatusCode::INTERNAL_SERVER_ERROR, "contact autograder responsible")
    }
}

/// Response to send back upon a submission.
#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub struct SubmitResponse {
    /// Human-readable detail.
    message: String,
    /// Request path.
    path: String,
    /// ID of the created submission, if one was registered.
    submission_id: Option<i64>,
}

impl SubmitResponse {
    pub fn new(req: &HttpRequest, msg: &str, submission_id: i64) -> SubmitResponse {
        SubmitResponse {
            message: msg.to_string(),
            path: req.path().to_string(),
            submission_id: Some(submission_id),
        }
    }
    pub fn without_id(req: &HttpRequest, msg: &str) -> SubmitResponse {
        SubmitResponse {
            message: msg.to_string(),
            path: req.path().to_string(),
            submission_id: None,
        }
    }
    /// 201 with the new submission's location when one was registered; a plain
    /// 200 acknowledgement when the webhook created nothing.
    pub fn to_http(&self) -> HttpResponse {
        match self.submission_id {
            // The submission is served from the same scope this webhook is
            // mounted under: `/api/submit/github` -> `/api/submission/{id}`.
            Some(id) => {
                let scope =
                    self.path.rsplit_once("/submit/").map(|(scope, _)| scope).unwrap_or_default();
                HttpResponse::Created()
                    .insert_header((header::LOCATION, format!("{scope}/submission/{id}")))
                    .json(self)
            }
            None => HttpResponse::Ok().json(self),
        }
    }
}

/// The outcome of grading one tag.
#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub struct JobStatusResponse {
    code: i32,
    text: String,
    finished: bool,
    /// `true` only for a successful job, `false` for any other terminal
    /// outcome, and `null` while the job has not finished.
    successful: Option<bool>,
}

impl From<JobStatus> for JobStatusResponse {
    fn from(status: JobStatus) -> Self {
        JobStatusResponse {
            code: status as i32,
            text: status.to_string(),
            finished: status.is_finished(),
            successful: status.is_finished().then(|| status == JobStatus::Success),
        }
    }
}

/// One graded tag of a submission.
#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub struct SubmissionJobResponse<'a> {
    tag: &'a str,
    /// The names the submitter asked for that resolved to `tag`.
    requested_as: &'a [String],
    status: JobStatusResponse,
    /// When the job becomes claimable, if it is being held back.
    #[schema(value_type = Option<String>, format = DateTime)]
    eligible_at: Option<&'a DateTime<Utc>>,
    /// Set if the job reached its status without ever being graded.
    #[schema(value_type = Option<String>, format = DateTime)]
    voided_at: Option<&'a DateTime<Utc>>,
    #[schema(value_type = Option<String>, format = DateTime)]
    started_at: Option<&'a DateTime<Utc>>,
    #[schema(value_type = Option<String>, format = DateTime)]
    finished_at: Option<&'a DateTime<Utc>>,
}

impl<'a> SubmissionJobResponse<'a> {
    pub fn new(job: &'a SubmissionJobPlain) -> Self {
        SubmissionJobResponse {
            tag: &job.tag,
            requested_as: &job.requested_as,
            status: job.status.into(),
            eligible_at: job.eligible_at.as_ref(),
            voided_at: job.voided_at.as_ref(),
            started_at: job.started_at.as_ref(),
            finished_at: job.finished_at.as_ref(),
        }
    }
}

/// A graded tag together with the report it produced.
///
/// # Serialization Note
/// Serialises as one object where the report sits alongside the job's fields
/// from `SubmissionJobResponse`.
#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub struct SubmissionJobWithReportResponse<'a> {
    #[serde(flatten)]
    job: SubmissionJobResponse<'a>,
    /// Full grading report once available. Its structure is documented
    /// separately; treated as an opaque object here.
    #[schema(value_type = Option<Object>)]
    report: Option<&'a Report>,
}

impl<'a> SubmissionJobWithReportResponse<'a> {
    pub fn to_http(&self) -> HttpResponse {
        HttpResponse::Ok().json(self)
    }

    pub fn new(entry: &'a SubmissionJobWithReport) -> Self {
        SubmissionJobWithReportResponse {
            job: SubmissionJobResponse::new(&entry.job),
            report: entry.report.as_ref(),
        }
    }
}

/// Information about a submission, to be sent back upon successful request.
#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub struct SubmissionResponse<'a> {
    path: String,
    submission_id: i64,
    /// The raw tag names the submitter asked for, before resolution.
    requested_tags: &'a [String],
    #[schema(value_type = String, format = DateTime)]
    submitted_at: &'a DateTime<Utc>,
    /// One entry per tag being graded. Empty if nothing about the submission
    /// could be graded, in which case the `report` field in
    /// `SubmissionResponse` will state the reason for this.
    jobs: Vec<SubmissionJobWithReportResponse<'a>>,
    /// Set only for exceptional circumstances, usually when no jobs could be
    /// created.
    #[schema(value_type = Option<Object>)]
    report: Option<&'a Report>,
    /// Submission origin
    origin: SubmissionResponseOrigin<'a>,
}

impl<'a> SubmissionResponse<'a> {
    pub fn new(req: &HttpRequest, sub: &'a SubmissionWithReports) -> SubmissionResponse<'a> {
        SubmissionResponse {
            path: req.path().to_string(),
            submission_id: sub.id,
            requested_tags: &sub.requested_tags,
            submitted_at: &sub.submitted_at,
            jobs: sub.jobs.iter().map(SubmissionJobWithReportResponse::new).collect(),
            report: sub.report.as_ref(),
            origin: SubmissionResponseOrigin::new(&sub.origin),
        }
    }
    pub fn to_http(&self) -> HttpResponse {
        HttpResponse::Ok().json(self)
    }
}

/// Information about the origin to be included with a response.
#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub enum SubmissionResponseOrigin<'a> {
    #[serde(rename = "github")]
    GitHub {
        domain: &'a str,
        repo: &'a str,
        org: &'a str,
        ssh_url: &'a str,
        user: &'a str,
        commit: &'a str,
    },
    #[serde(rename = "gitlab")]
    GitLab {
        domain: &'a str,
        repo: &'a str,
        namespace: &'a str,
        ssh_url: &'a str,
        user: &'a str,
        commit: &'a str,
    },
}
impl<'a> SubmissionResponseOrigin<'a> {
    pub fn new(origin: &'a SubmissionOrigin) -> Self {
        match &origin.origin {
            StoredOriginEnum::GitHub(gh) => Self::GitHub {
                domain: &gh.src.domain,
                repo: &gh.src.repo,
                org: &gh.src.org,
                ssh_url: &gh.src.ssh_url,
                user: &gh.info.user,
                commit: &gh.info.commit,
            },
            StoredOriginEnum::GitLab(gl) => Self::GitLab {
                domain: &gl.src.domain,
                repo: &gl.src.repo,
                namespace: &gl.src.namespace,
                ssh_url: &gl.src.ssh_url,
                user: &gl.info.user,
                commit: &gl.info.commit,
            },
        }
    }
}

/// Submissions retrieved submissions from a search query.
#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub struct SubmissionSearchResponse<'a> {
    pub path: String,
    pub items: &'a [SubmissionSearchResponseItem<'a>],
}

impl<'a> SubmissionSearchResponse<'a> {
    pub fn to_http(&self) -> HttpResponse {
        HttpResponse::Ok().json(self)
    }
}

/// Response items to a search query. This will never include the grading
/// report, which can only be retrieved when querying a submission directly.
#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub struct SubmissionSearchResponseItem<'a> {
    pub submission_id: i64,
    requested_tags: &'a [String],
    #[schema(value_type = String, format = DateTime)]
    submitted_at: &'a DateTime<Utc>,
    jobs: Vec<SubmissionJobResponse<'a>>,
    origin: SubmissionResponseOrigin<'a>,
}

impl<'a> SubmissionSearchResponseItem<'a> {
    pub fn new(sub: &'a Submission) -> Self {
        SubmissionSearchResponseItem {
            submission_id: sub.id,
            requested_tags: &sub.requested_tags,
            submitted_at: &sub.submitted_at,
            jobs: sub.jobs.iter().map(SubmissionJobResponse::new).collect(),
            origin: SubmissionResponseOrigin::new(&sub.origin),
        }
    }
}

/// Information the grading tags
#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub struct TagListResponse {
    path: String,
    tags: BTreeMap<String, TagListDetails>,
    /// Everything resolvable that is not a tag in its own right.
    tag_groups: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub struct TagListDetails {
    build: BuildConfig,
    /// Arbitrary tag metadata, opaque to the autograder.
    #[schema(value_type = Object)]
    metadata: BTreeMap<String, serde_json::Value>,
    has_task: bool,
}

impl TagListResponse {
    pub fn new(req: &HttpRequest, tc: &Tests) -> TagListResponse {
        TagListResponse {
            path: req.path().to_string(),
            tags: tc
                .tags
                .iter()
                .map(|(name, t)| (name.clone(), TagListDetails::from_tag(t)))
                .collect(),
            tag_groups: tc
                .tag_resolution
                .iter()
                .filter(|(name, _)| !tc.tags.contains_key(*name))
                .map(|(name, tagnames)| (name.clone(), tagnames.clone()))
                .collect(),
        }
    }
    pub fn to_http(&self) -> HttpResponse {
        HttpResponse::Ok().json(self)
    }
}

impl TagListDetails {
    /// Creates a details struct for the given tag.
    fn from_tag(t: &Tag) -> TagListDetails {
        TagListDetails {
            build: t.build.clone(),
            metadata: t
                .metadata
                .clone()
                .into_iter()
                .map(|(k, v)| (k, v.deserialize_into().unwrap_or(serde_json::Value::Null)))
                .collect(),
            has_task: t.task_file.is_some(),
        }
    }
}

/// Information about a single grading tag. If this is an alias, this will return
/// all the tags that it will grade.
#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub struct TagResponse {
    path: String,
    tags: BTreeMap<String, TagListDetails>,
}

impl TagResponse {
    /// Attempts to construct a tag response, returning `None` if the tag_name
    /// is not found.
    pub fn new(req: &HttpRequest, tc: &Tests, tag_name: &str) -> Option<TagResponse> {
        Some(TagResponse {
            path: req.path().to_string(),
            tags: tc
                .tag_resolution
                .get(tag_name)?
                .iter()
                .filter_map(|name| {
                    Some((name.to_owned(), TagListDetails::from_tag(tc.tags.get(name)?)))
                })
                .collect(),
        })
    }
    pub fn to_http(&self) -> HttpResponse {
        HttpResponse::Ok().json(self)
    }
}
