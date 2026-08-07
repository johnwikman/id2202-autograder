use std::{collections::BTreeMap, time::SystemTime};

use actix_web::{
    http::{header, StatusCode},
    HttpRequest, HttpResponse,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use derive_more::derive::{Display, Error};
use num_traits::FromPrimitive;

use id2202_autograder::{
    config::{Tag, TagBuildConfig, Tests},
    db::models::{
        Submission, SubmissionInfoGitHub, SubmissionInfoGitLab, SubmissionSourceGitHub,
        SubmissionSourceGitLab, SubmissionWithReport,
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
        HttpResponse::build(self.status_code())
            .content_type("application/problem+json")
            .json(self)
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
        Self::new(
            req,
            StatusCode::INTERNAL_SERVER_ERROR,
            "contact autograder responsible",
        )
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
                let scope = self
                    .path
                    .rsplit_once("/submit/")
                    .map(|(scope, _)| scope)
                    .unwrap_or_default();
                HttpResponse::Created()
                    .insert_header((header::LOCATION, format!("{scope}/submission/{id}")))
                    .json(self)
            }
            None => HttpResponse::Ok().json(self),
        }
    }
}

/// Information about a submission, to be sent back upon successful request.
#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub struct SubmissionResponse<'a> {
    path: String,
    submission_id: i64,
    grading_tags: &'a [String],
    finished: bool,
    successful: Option<bool>,
    #[schema(value_type = String, format = DateTime)]
    date_submitted: &'a SystemTime,
    #[schema(value_type = Option<String>, format = DateTime)]
    date_exec_started: Option<&'a SystemTime>,
    #[schema(value_type = Option<String>, format = DateTime)]
    date_exec_finished: Option<&'a SystemTime>,
    /// Full grading report once available. Its structure is documented
    /// separately; treated as an opaque object here.
    #[schema(value_type = Option<Object>)]
    report: Option<Report>,
    /// Submission source
    source: SubmissionResponseSource<'a>,
}

impl<'a> SubmissionResponse<'a> {
    pub fn new(
        req: &HttpRequest,
        sub: &'a SubmissionWithReport,
        source: SubmissionResponseSource<'a>,
    ) -> SubmissionResponse<'a> {
        use id2202_autograder::db::models::SubmissionStatusCode as SSC;

        SubmissionResponse {
            path: req.path().to_string(),
            submission_id: sub.id,
            grading_tags: &sub.grading_tags,
            finished: sub.exec_finished,
            successful: if sub.exec_finished {
                SSC::from_i32(sub.exec_status_code).map(|c| c == SSC::Success)
            } else {
                None
            },
            date_submitted: &sub.date_submitted,
            date_exec_started: sub.exec_date_started.as_ref(),
            date_exec_finished: sub.exec_date_finished.as_ref(),
            report: sub
                .exec_report
                .as_ref()
                .and_then(|v| Report::deserialize(v).ok()),
            source,
        }
    }
    pub fn to_http(&self) -> HttpResponse {
        HttpResponse::Ok().json(self)
    }
}

/// Information about the source to be included with a response.
#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub enum SubmissionResponseSource<'a> {
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
impl<'a> SubmissionResponseSource<'a> {
    pub fn new_github(
        gh_src: &'a SubmissionSourceGitHub,
        gh_info: &'a SubmissionInfoGitHub,
    ) -> Self {
        Self::GitHub {
            domain: &gh_src.domain,
            repo: &gh_src.repo,
            org: &gh_src.org,
            ssh_url: &gh_src.ssh_url,
            user: &gh_info.user,
            commit: &gh_info.commit,
        }
    }
    pub fn new_gitlab(
        gl_src: &'a SubmissionSourceGitLab,
        gl_info: &'a SubmissionInfoGitLab,
    ) -> Self {
        Self::GitLab {
            domain: &gl_src.domain,
            repo: &gl_src.repo,
            namespace: &gl_src.namespace,
            ssh_url: &gl_src.ssh_url,
            user: &gl_info.user,
            commit: &gl_info.commit,
        }
    }
}

/// Submissions retrieved submissions from a search query.
#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub struct SubmissionSearchResponse<'a> {
    pub path: String,
    pub items: Vec<SubmissionSearchResponseItem<'a>>,
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
    submission_id: i64,
    grading_tags: &'a [String],
    finished: bool,
    #[schema(value_type = String, format = DateTime)]
    date_submitted: &'a SystemTime,
    source: SubmissionResponseSource<'a>,
}

impl<'a> SubmissionSearchResponseItem<'a> {
    pub fn github_from_db(
        sub: &'a Submission,
        gh_info: &'a SubmissionInfoGitHub,
        gh_src: &'a SubmissionSourceGitHub,
    ) -> Self {
        SubmissionSearchResponseItem {
            submission_id: sub.id,
            grading_tags: &sub.grading_tags,
            finished: sub.exec_finished,
            date_submitted: &sub.date_submitted,
            source: SubmissionResponseSource::new_github(gh_src, gh_info),
        }
    }
    pub fn gitlab_from_db(
        sub: &'a Submission,
        gl_info: &'a SubmissionInfoGitLab,
        gl_src: &'a SubmissionSourceGitLab,
    ) -> Self {
        SubmissionSearchResponseItem {
            submission_id: sub.id,
            grading_tags: &sub.grading_tags,
            finished: sub.exec_finished,
            date_submitted: &sub.date_submitted,
            source: SubmissionResponseSource::new_gitlab(gl_src, gl_info),
        }
    }
}

/// Information the grading tags
#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub struct TagListResponse {
    path: String,
    tags: BTreeMap<String, TagListDetails>,
    tag_groups: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Serialize, JsonSchema, ToSchema)]
pub struct TagListDetails {
    build: TagBuildConfig,
    /// Arbitrary tag metadata, opaque to the autograder.
    #[schema(value_type = Object)]
    metadata: BTreeMap<String, serde_json::Value>,
    has_task: bool,
}

impl TagListResponse {
    pub fn new(req: &HttpRequest, tc: &Tests) -> TagListResponse {
        let mut r = TagListResponse {
            path: req.path().to_string(),
            tags: BTreeMap::new(),
            tag_groups: BTreeMap::new(),
        };

        for (group_name, tags) in &tc.tag_groups {
            match tags.as_slice() {
                [t] => {
                    if group_name == &t.name {
                        r.tags.insert(t.name.clone(), TagListDetails::from_tag(t));
                    } else {
                        r.tag_groups
                            .insert(group_name.clone(), vec![t.name.clone()]);
                    }
                }
                _ => {
                    r.tag_groups.insert(
                        group_name.clone(),
                        tags.iter().map(|t| t.name.clone()).collect(),
                    );
                }
            }
        }

        r
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

/// Information the a single grading tag. If this is an alias, this will return
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
                .tag_groups
                .get(tag_name)?
                .iter()
                .map(|t| (t.name.to_string(), TagListDetails::from_tag(t)))
                .collect(),
        })
    }
    pub fn to_http(&self) -> HttpResponse {
        HttpResponse::Ok().json(self)
    }
}
