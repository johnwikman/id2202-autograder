use actix_web::{
    body::MessageBody,
    dev::{ServiceRequest, ServiceResponse},
    middleware::Next,
    web::{self, ServiceConfig},
    HttpMessage, HttpRequest, HttpResponse, ResponseError,
};

use crate::auth::AuthorizationInfo;
use id2202_autograder::config::Settings;

mod common;
mod response;
mod submission;
mod submit_github;
mod submit_gitlab;
mod tag;

/// Configuration for the API services.
///
/// This is expected to be wrapped in an actix scope, such that everything is
/// prefixed with "/api", or similar.
pub fn config(cfg: &mut ServiceConfig, _settings: &Settings) {
    cfg.default_service(web::to(not_found));

    cfg.route(
        "/submit/github",
        web::post().to(submit_github::github_submission),
    );

    cfg.route(
        "/submit/gitlab",
        web::post().to(submit_gitlab::gitlab_submit_webhook),
    );

    cfg.route(
        "/submission/{id}",
        web::get().to(submission::get_submission),
    );

    cfg.route("/tag", web::get().to(tag::get_taglist));
    cfg.route("/tag/{tagname}", web::get().to(tag::get_tag));
    cfg.route("/tag/{tagname}/task", web::get().to(tag::get_tag_task));

    // JSON schemas for responses
    cfg.route(
        "/schema/error",
        web::get().to(response::schema_callback!(response::ErrorResponse)),
    );

    cfg.route(
        "/schema/submit",
        web::get().to(response::schema_callback!(response::SubmitResponse)),
    );

    cfg.route(
        "/schema/submission",
        web::get().to(response::schema_callback!(response::SubmissionResponse)),
    );

    cfg.route(
        "/schema/tag-list",
        web::get().to(response::schema_callback!(response::TagListResponse)),
    );
    cfg.route(
        "/schema/tag",
        web::get().to(response::schema_callback!(response::TagResponse)),
    );
}

/// "404: Not found" response for API requests.
async fn not_found(req: HttpRequest) -> Result<HttpResponse, actix_web::Error> {
    use response::ErrorResponse;

    Ok(ErrorResponse::not_found(&req, "API resource could not be found").error_response())
}

/// Middleware for forcing a request to be authenticated with the API key,
/// except if the path starts with any of the provided prefixes.
pub async fn auth_hook(
    scope_prefix: &str,
    req: ServiceRequest,
    next: Next<impl MessageBody>,
) -> Result<ServiceResponse<impl MessageBody>, actix_web::Error> {
    // TODO: Want the have the "/api" bit to come from the actix scope.
    static BYPASS_PREFIXES: [&str; 1] = ["/submit"];

    if !BYPASS_PREFIXES
        .iter()
        .any(|pfx| req.path().split_at_checked(scope_prefix.len()).map_or(false, |(_, p)| p.starts_with(pfx)))
    {
        let auth_info = req
            .extensions()
            .get::<AuthorizationInfo>()
            .ok_or_else(|| {
                response::ErrorResponse::unauthorized(req.request(), "missing Authorization header")
            })?
            .clone();
        if !auth_info.api_auth_ok {
            // API authentication failed
            return Err(response::ErrorResponse::unauthorized(
                req.request(),
                "API authentication failed",
            )
            .into());
        }
    }

    next.call(req).await
}
