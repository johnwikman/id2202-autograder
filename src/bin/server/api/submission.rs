use actix_web::{
    get,
    web::{self},
    HttpRequest, Responder,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use id2202_autograder::{
    config::Settings,
    db::{
        conn::DatabaseConnection,
        models::raw::SubmissionRow,
        models::{Submission, SubmissionWithReports},
    },
    utils::deserialize_iso8601,
};
use utoipa::IntoParams;

use crate::api::response::{
    ErrorResponse, SubmissionJobWithReportResponse, SubmissionResponse, SubmissionSearchResponse,
    SubmissionSearchResponseItem,
};

/// Fetches detailed information about a single submission from the database
/// that matches the provided id.
#[utoipa::path(
    tag = "Submissions",
    params(("id" = i64, Path, description = "ID of the submission to fetch.")),
    security(("api_token" = [])),
    responses(
        (status = 200, description = "Submission info, including the grading report once finished", body = SubmissionResponse),
        (status = 400, description = "Malformed submission ID", body = ErrorResponse),
        (status = 404, description = "No submission with that ID", body = ErrorResponse),
    ),
)]
#[get("/submission/{id}")]
pub async fn get_submission(
    data: web::Data<Settings>,
    req: HttpRequest,
    submission_id: web::Path<String>,
) -> Result<impl Responder, actix_web::Error> {
    let settings = data.get_ref();

    // Request is Authorized
    let parsed_id: i64 = match submission_id.parse() {
        Ok(v) => v,
        Err(_) => {
            log::error!("Bad submission id: {submission_id}");
            return Err(ErrorResponse::bad_request(&req, "bad submission id format").into());
        }
    };

    let mut conn = match DatabaseConnection::connect(settings) {
        Ok(conn) => conn,
        Err(e) => {
            log::error!("Could not open database connection: {e}");
            return Err(ErrorResponse::internal_server_error(&req).into());
        }
    };

    let sub = SubmissionWithReports::by_id_opt(&mut conn, parsed_id)
        .map_err(|e| {
            log::error!("could not get submission {parsed_id} from database: {e}");
            ErrorResponse::internal_server_error(&req)
        })?
        .ok_or_else(|| ErrorResponse::not_found(&req, "submission not found"))?;

    Ok(SubmissionResponse::new(&req, &sub).to_http())
}

/// Fetches a single graded tag of a submission, so that one tag's report can be
/// read without pulling every report the submission produced.
#[utoipa::path(
    tag = "Submissions",
    params(
        ("id" = i64, Path, description = "ID of the submission the job belongs to."),
        ("tag" = String, Path, description = "The grading tag that the job graded."),
    ),
    security(("api_token" = [])),
    responses(
        (status = 200, description = "The job, including its grading report once finished", body = SubmissionJobWithReportResponse),
        (status = 400, description = "Malformed submission ID", body = ErrorResponse),
        (status = 404, description = "No submission with that ID, or it has no job for that tag", body = ErrorResponse),
    ),
)]
#[get("/submission/{id}/job/{tag}")]
pub async fn get_submission_job(
    data: web::Data<Settings>,
    req: HttpRequest,
    path: web::Path<(String, String)>,
) -> Result<impl Responder, actix_web::Error> {
    let settings = data.get_ref();
    let (submission_id, tag) = path.into_inner();

    let parsed_id: i64 = match submission_id.parse() {
        Ok(v) => v,
        Err(_) => {
            log::error!("Bad submission id: {submission_id}");
            return Err(ErrorResponse::bad_request(&req, "bad submission id format").into());
        }
    };

    let mut conn = match DatabaseConnection::connect(settings) {
        Ok(conn) => conn,
        Err(e) => {
            log::error!("Could not open database connection: {e}");
            return Err(ErrorResponse::internal_server_error(&req).into());
        }
    };

    let sub = SubmissionWithReports::by_id_opt(&mut conn, parsed_id)
        .map_err(|e| {
            log::error!("could not get submission {parsed_id} from database: {e}");
            ErrorResponse::internal_server_error(&req)
        })?
        .ok_or_else(|| ErrorResponse::not_found(&req, "submission not found"))?;

    // A tag the submission never had a job for is a 404 rather than an empty
    // result, so that a typo is distinguishable from a tag that has not run.
    let job = sub
        .jobs
        .iter()
        .find(|entry| entry.job.tag == tag)
        .ok_or_else(|| ErrorResponse::not_found(&req, "no job for that tag"))?;

    Ok(SubmissionJobWithReportResponse::new(job).to_http())
}

/// Query parameters for get_submission_search.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct SubmissionSearchFilterQuery {
    /// Kind of submission source: `github` or `gitlab`
    source_kind: Option<String>,
    /// The git commit hash associated with the submission.
    commit_hash: Option<String>,
    /// The username associated with the submission.
    user: Option<String>,
    /// The repository associated with the submission.
    repo: Option<String>,
    /// Only include submissions that has been graded with this tag. This does
    /// not capture tags which are aliases (that resolve to one or more grading
    /// tags).
    tag: Option<String>,
    /// Only include submissions that has submitted after this date (inclusive).
    #[serde(default, deserialize_with = "deserialize_iso8601")]
    #[param(value_type = String, format = DateTime)]
    after: Option<DateTime<Utc>>,
    /// Only include submissions that has submitted before this date (inclusive).
    #[serde(default, deserialize_with = "deserialize_iso8601")]
    #[param(value_type = String, format = DateTime)]
    before: Option<DateTime<Utc>>,
    /// Maximum number of returned results. Defaults to 100 if not specified.
    limit: Option<u32>,
}

/// Searches for submissions in the database, using one or more specified
/// filters to narrow the search space.
#[utoipa::path(
    tag = "Submissions",
    params(SubmissionSearchFilterQuery),
    security(("api_token" = [])),
    responses(
        (status = 200, description = "Submission info, including the grading report once finished", body = SubmissionSearchResponse),
        (status = 400, description = "Incorrect formatting of query keys, or invalid source kind.", body = ErrorResponse),
    ),
)]
#[get("/submission")]
pub async fn get_submission_search(
    data: web::Data<Settings>,
    req: HttpRequest,
) -> Result<impl Responder, actix_web::Error> {
    use diesel::{self, ExpressionMethods, QueryDsl, RunQueryDsl, SelectableHelper};
    use id2202_autograder::db::schema::{
        submission_info_github::{self, columns as gh_info_col},
        submission_info_gitlab::{self, columns as gl_info_col},
        submission_jobs::{self, columns as job_col},
        submission_origin_github::{self, columns as gh_src_col},
        submission_origin_gitlab::{self, columns as gl_src_col},
        submissions::{self, columns as sub_col},
    };

    let settings = data.get_ref();

    let q = match actix_web::web::Query::<SubmissionSearchFilterQuery>::from_query(
        req.query_string(),
    ) {
        Ok(q) => q.into_inner(),
        Err(_) => {
            return Err(ErrorResponse::bad_request(&req, "bad parameters").into());
        }
    };

    let limit = q.limit.unwrap_or(100);

    let mut conn = match DatabaseConnection::connect(settings) {
        Ok(conn) => conn,
        Err(e) => {
            log::error!("Could not open database connection: {e}");
            return Err(ErrorResponse::internal_server_error(&req).into());
        }
    };

    struct SearchSources {
        github: bool,
        gitlab: bool,
    }
    let search_sources = match q.source_kind.map(|s| s.to_lowercase()).as_deref() {
        Some("github") => SearchSources { github: true, gitlab: false },
        Some("gitlab") => SearchSources { github: false, gitlab: true },
        Some(kind) => {
            return Err(ErrorResponse::bad_request(
                &req,
                &format!("invalid source kind \"{}\"", kind),
            )
            .into());
        }
        None => SearchSources { github: true, gitlab: true },
    };

    macro_rules! apply_common_filters {
        ($dbq:ident, $q:ident) => {
            if let Some(tag) = &$q.tag {
                $dbq = $dbq.filter(
                    sub_col::id.eq_any(
                        submission_jobs::table
                            .select(job_col::submission_id)
                            .filter(job_col::tag.eq(tag)),
                    ),
                );
            }
            if let Some(after) = &$q.after {
                $dbq = $dbq.filter(sub_col::submitted_at.ge(after));
            }
            if let Some(before) = &$q.before {
                $dbq = $dbq.filter(sub_col::submitted_at.le(before));
            }
        };
    }

    let gh_results = if search_sources.github {
        let mut dbq = submissions::table
            .inner_join(submission_info_github::table.inner_join(submission_origin_github::table))
            .select(SubmissionRow::as_select())
            .into_boxed();

        apply_common_filters!(dbq, q);

        if let Some(commit_hash) = &q.commit_hash {
            dbq = dbq.filter(gh_info_col::commit.eq(commit_hash));
        }
        if let Some(user) = &q.user {
            dbq = dbq.filter(gh_info_col::user.eq(user));
        }
        if let Some(repo) = &q.repo {
            dbq = dbq.filter(gh_src_col::repo.eq(repo));
        }

        let found: Vec<SubmissionRow> =
            dbq.order(sub_col::id.desc()).limit(limit.into()).load(&mut conn.conn).map_err(
                |e| {
                    log::error!("Could not fetch results from database: {e}");
                    ErrorResponse::internal_server_error(&req)
                },
            )?;

        found
    } else {
        Vec::new()
    };

    let gl_results = if search_sources.gitlab {
        let mut dbq = submissions::table
            .inner_join(submission_info_gitlab::table.inner_join(submission_origin_gitlab::table))
            .select(SubmissionRow::as_select())
            .into_boxed();

        apply_common_filters!(dbq, q);

        if let Some(commit_hash) = &q.commit_hash {
            dbq = dbq.filter(gl_info_col::commit.eq(commit_hash));
        }
        if let Some(user) = &q.user {
            dbq = dbq.filter(gl_info_col::user.eq(user));
        }
        if let Some(repo) = &q.repo {
            dbq = dbq.filter(gl_src_col::repo.eq(repo));
        }

        let found: Vec<SubmissionRow> =
            dbq.order(sub_col::id.desc()).limit(limit.into()).load(&mut conn.conn).map_err(
                |e| {
                    log::error!("Could not fetch results from database: {e}");
                    ErrorResponse::internal_server_error(&req)
                },
            )?;

        found
    } else {
        Vec::new()
    };

    let mut rows = gh_results;
    rows.extend(gl_results);
    rows.sort_by_key(|r| std::cmp::Reverse(r.id));
    rows.truncate(limit as usize);

    let subs = Submission::from_rows(&mut conn, rows).map_err(|e| {
        log::error!("Could not instantiate the submissions: {e}");
        ErrorResponse::internal_server_error(&req)
    })?;

    let results: Vec<SubmissionSearchResponseItem> =
        subs.iter().map(SubmissionSearchResponseItem::new).collect();

    Ok(SubmissionSearchResponse { path: req.path().to_string(), items: &results }.to_http())
}
