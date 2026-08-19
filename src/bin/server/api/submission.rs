use actix_web::{
    get,
    web::{self},
    HttpRequest, Responder,
};
use num_traits::FromPrimitive;
use serde::Deserialize;

use diesel::OptionalExtension;
use id2202_autograder::{
    config::Settings,
    db::{
        conn::DatabaseConnection,
        models::{
            Submission, SubmissionInfoGitHub, SubmissionInfoGitLab, SubmissionSource,
            SubmissionSourceGitHub, SubmissionSourceGitLab, SubmissionSourceKind,
            SubmissionWithReport,
        },
    },
};
use utoipa::IntoParams;

use crate::api::response::{
    ErrorResponse, SubmissionResponse, SubmissionResponseSource, SubmissionSearchResponse,
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
    use diesel::{self, ExpressionMethods, QueryDsl, RunQueryDsl, SelectableHelper};
    use id2202_autograder::db::schema::{
        submission_info_github::{self, columns as gh_info_col},
        submission_info_gitlab::{self, columns as gl_info_col},
        submission_source_github, submission_source_gitlab, submission_sources,
        submissions::{self, columns as sub_col},
    };

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

    let (swr, subsrc): (SubmissionWithReport, SubmissionSource) = submissions::table
        .inner_join(submission_sources::table)
        .select((SubmissionWithReport::as_select(), SubmissionSource::as_select()))
        .filter(sub_col::id.eq(parsed_id))
        .first(&mut conn.conn)
        .optional()
        .map_err(|e: diesel::result::Error| {
            log::error!("could not get submission {parsed_id} with report from database: {:?}", e);
            ErrorResponse::internal_server_error(&req)
        })?
        .ok_or_else(|| ErrorResponse::not_found(&req, "submission not found"))?;

    let srckind = SubmissionSourceKind::from_i32(subsrc.kind).ok_or_else(|| {
        log::error!("got invalid source kind {} for submission {parsed_id}", subsrc.kind);
        ErrorResponse::internal_server_error(&req)
    })?;
    match srckind {
        SubmissionSourceKind::GitHub => {
            let (gh_src, gh_info) = submission_source_github::table.inner_join(submission_info_github::table)
                .select((SubmissionSourceGitHub::as_select(), SubmissionInfoGitHub::as_select()))
                .filter(gh_info_col::submission_id.eq(swr.id))
                .first(&mut conn.conn)
                .map_err(|e: diesel::result::Error| {
                    log::error!(
                        "could not get GitHub source information for submission {parsed_id} from database: {:?}",
                        e
                    );
                    ErrorResponse::internal_server_error(&req)
                })?;
            Ok(SubmissionResponse::new(
                &req,
                &swr,
                SubmissionResponseSource::new_github(&gh_src, &gh_info),
            )
            .to_http())
        }
        SubmissionSourceKind::GitLab => {
            let (gl_src, gl_info) = submission_source_gitlab::table.inner_join(submission_info_gitlab::table)
                .select((SubmissionSourceGitLab::as_select(), SubmissionInfoGitLab::as_select()))
                .filter(gl_info_col::submission_id.eq(swr.id))
                .first(&mut conn.conn)
                .map_err(|e: diesel::result::Error| {
                    log::error!(
                        "could not get GitLab source information for submission {parsed_id} from database: {:?}",
                        e
                    );
                    ErrorResponse::internal_server_error(&req)
                })?;
            Ok(SubmissionResponse::new(
                &req,
                &swr,
                SubmissionResponseSource::new_gitlab(&gl_src, &gl_info),
            )
            .to_http())
        }
    }
}

/// Query parameters for get_submission_search.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct SubmissionSearchFilterQuery {
    /// Kind of submission source: `github` or `gitlab`
    source_kind: String,
    /// The git commit hash associated with the submission.
    commit_hash: Option<String>,
    /// The username associated with the submission.
    user: Option<String>,
    /// The repository associated with the submission.
    repo: Option<String>,
}

/// Searches for submissions in the database, using one or more specified
/// filters to narrow the search space.
#[utoipa::path(
    tag = "Submissions",
    params(SubmissionSearchFilterQuery),
    security(("api_token" = [])),
    responses(
        (status = 200, description = "Submission info, including the grading report once finished", body = SubmissionSearchResponse),
        (status = 400, description = "Missing required query keys, or invalid source kind.", body = ErrorResponse),
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
        submission_source_github::{self, columns as gh_src_col},
        submission_source_gitlab::{self, columns as gl_src_col},
        submissions::{self, columns as sub_col},
    };

    let settings = data.get_ref();

    let q = match actix_web::web::Query::<SubmissionSearchFilterQuery>::from_query(
        req.query_string(),
    ) {
        Ok(q) => q.into_inner(),
        Err(_) => {
            return Err(ErrorResponse::bad_request(&req, "missing required parameters").into());
        }
    };

    let mut conn = match DatabaseConnection::connect(settings) {
        Ok(conn) => conn,
        Err(e) => {
            log::error!("Could not open database connection: {e}");
            return Err(ErrorResponse::internal_server_error(&req).into());
        }
    };

    match q.source_kind.to_lowercase().as_str() {
        "github" => {
            let mut dbq = submissions::table
                .inner_join(
                    submission_info_github::table.inner_join(submission_source_github::table),
                )
                .select((
                    Submission::as_select(),
                    SubmissionInfoGitHub::as_select(),
                    SubmissionSourceGitHub::as_select(),
                ))
                .into_boxed();

            if let Some(commit_hash) = &q.commit_hash {
                dbq = dbq.filter(gh_info_col::commit.eq(commit_hash));
            }
            if let Some(user) = &q.user {
                dbq = dbq.filter(gh_info_col::user.eq(user));
            }
            if let Some(repo) = &q.repo {
                dbq = dbq.filter(gh_src_col::repo.eq(repo));
            }

            let found: Vec<(Submission, SubmissionInfoGitHub, SubmissionSourceGitHub)> =
                dbq.order(sub_col::id.desc()).limit(100).load(&mut conn.conn).map_err(|e| {
                    log::error!("Could not fetch results from database: {e}");
                    ErrorResponse::internal_server_error(&req)
                })?;

            Ok(SubmissionSearchResponse {
                path: req.path().to_string(),
                items: found
                    .iter()
                    .map(|(sub, info, src)| {
                        SubmissionSearchResponseItem::github_from_db(sub, info, src)
                    })
                    .collect(),
            }
            .to_http())
        }
        "gitlab" => {
            let mut dbq = submissions::table
                .inner_join(
                    submission_info_gitlab::table.inner_join(submission_source_gitlab::table),
                )
                .select((
                    Submission::as_select(),
                    SubmissionInfoGitLab::as_select(),
                    SubmissionSourceGitLab::as_select(),
                ))
                .into_boxed();

            if let Some(commit_hash) = &q.commit_hash {
                dbq = dbq.filter(gl_info_col::commit.eq(commit_hash));
            }
            if let Some(user) = &q.user {
                dbq = dbq.filter(gl_info_col::user.eq(user));
            }
            if let Some(repo) = &q.repo {
                dbq = dbq.filter(gl_src_col::repo.eq(repo));
            }

            let found: Vec<(Submission, SubmissionInfoGitLab, SubmissionSourceGitLab)> =
                dbq.order(sub_col::id.desc()).limit(100).load(&mut conn.conn).map_err(|e| {
                    log::error!("Could not fetch results from database: {e}");
                    ErrorResponse::internal_server_error(&req)
                })?;

            Ok(SubmissionSearchResponse {
                path: req.path().to_string(),
                items: found
                    .iter()
                    .map(|(sub, info, src)| {
                        SubmissionSearchResponseItem::gitlab_from_db(sub, info, src)
                    })
                    .collect(),
            }
            .to_http())
        }
        _ => Err(ErrorResponse::bad_request(
            &req,
            &format!("invalid source kind \"{}\"", q.source_kind),
        )
        .into()),
    }
}
