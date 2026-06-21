use actix_web::{
    web::{self, ServiceConfig},
    HttpRequest, HttpResponse, ResponseError,
};

use id2202_autograder::config::Settings;

mod common;
mod response;
mod submission;
mod submit_github;
mod submit_gitlab;

/// Configuration for the API services.
///
/// The provided `api` can be the empty string or something like "/api". The
/// reason for this being configurable is to allow for future API versions,
/// such as "/api/v1", etc.
pub fn config(cfg: &mut ServiceConfig, _settings: &Settings, prefix: &str) {
    cfg.route(
        &format!("{prefix}/submit/github"),
        web::post().to(submit_github::github_submission),
    );

    cfg.route(
        &format!("{prefix}/submit/gitlab"),
        web::post().to(submit_gitlab::gitlab_submit_webhook),
    );

    cfg.route(
        &format!("{prefix}/submission/{}", "{id}"),
        web::get().to(submission::get_submission),
    );

    // JSON schemas for responses
    cfg.route(
        &format!("{prefix}/schema/error"),
        web::get().to(response::schema_callback!(response::ErrorResponse)),
    );

    cfg.route(
        &format!("{prefix}/schema/submit"),
        web::get().to(response::schema_callback!(response::SubmitResponse)),
    );

    cfg.route(
        &format!("{prefix}/schema/submission"),
        web::get().to(response::schema_callback!(response::SubmissionResponse)),
    );
}

/// "404: Not found" response for API requests.
/// This function is configured in main.rs
pub fn not_found(req: HttpRequest) -> Result<HttpResponse, actix_web::Error> {
    use response::ErrorResponse;

    Ok(ErrorResponse::not_found(&req, "API resource could not be found").error_response())
}
