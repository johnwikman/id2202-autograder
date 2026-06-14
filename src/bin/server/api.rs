use actix_web::{
    web::{self, ServiceConfig},
    HttpRequest, HttpResponse, ResponseError,
};

use id2202_autograder::config::Settings;

mod common;
mod response;
mod submit_github;
mod submit_gitlab;

/// Configuration for the API services.
///
/// The provided `api` can be the empty string or something like "/api". The
/// reason for this being configurable is to allow for future API versions,
/// such as "/api/v1", etc.
pub fn config(cfg: &mut ServiceConfig, settings: &Settings, prefix: &str) {
    let s = settings.clone();
    cfg.route(
        &format!("{prefix}/submit/github"),
        web::post().to(move |req, pl| submit_github::github_submission(s.clone(), req, pl)),
    );

    cfg.route(
        &format!("{prefix}/submit/gitlab"),
        web::post().to(submit_gitlab::gitlab_submit_webhook),
    );
}

/// "404: Not found" response for API requests.
/// This function is configured in main.rs
pub fn not_found(req: HttpRequest) -> Result<HttpResponse, actix_web::Error> {
    use response::ErrorResponse;

    Ok(ErrorResponse::not_found(&req, "API resource could not be found").error_response())
}
