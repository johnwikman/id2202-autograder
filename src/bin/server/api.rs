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
pub mod response;
pub mod submission;
pub mod submit_github;
pub mod submit_gitlab;
pub mod tag;

/// Configuration for the API services.
///
/// This is expected to be wrapped in an actix scope, such that everything is
/// prefixed with "/api", or similar.
pub fn config(cfg: &mut ServiceConfig, _settings: &Settings) {
    cfg.default_service(web::to(not_found));

    // Route paths for these handlers are defined on the handlers themselves via
    // actix route macros (`#[get(...)]` / `#[post(...)]`), which is also where
    // utoipa reads them for the generated OpenAPI documentation.
    cfg.service(submit_github::github_submission);
    cfg.service(submit_gitlab::gitlab_submit_webhook);
    cfg.service(submission::get_submission);
    cfg.service(submission::get_submission_search);
    cfg.service(tag::get_taglist);
    cfg.service(tag::get_tag);
    cfg.service(tag::get_tag_task);

    // JSON schemas for responses
    cfg.route("/schema/error", web::get().to(response::schema_callback!(response::ErrorResponse)));

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
    cfg.route("/schema/tag", web::get().to(response::schema_callback!(response::TagResponse)));
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
    static BYPASS_PREFIXES: [&str; 1] = ["/submit"];

    if !BYPASS_PREFIXES.iter().any(|pfx| {
        // We check the part that comes after the scope prefix, assuming that
        // the path starts with that.
        req.path().split_at_checked(scope_prefix.len()).is_some_and(|(_, p)| p.starts_with(pfx))
    }) {
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
