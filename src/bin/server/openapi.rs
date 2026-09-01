//! Assembles the OpenAPI specification for the REST API.
//!
//! This lives in the `server` binary because the `#[utoipa::path]` annotations
//! and the `paths(...)` list below reference the API handler functions, which
//! are private to this crate. Its sole job is to produce the OpenAPI **spec** as
//! JSON (via `server emit-openapi`); rendering that spec into the HTML
//! documentation page is done separately by the `docgen` binary.

use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use id2202_autograder::error::Error;

use crate::api::response::{
    ErrorResponse, SubmissionJobWithReportResponse, SubmissionResponse, SubmitResponse,
    TagListResponse, TagResponse,
};
use crate::api::{submission, submit_github, submit_gitlab, tag};

/// The API operations, collected from the annotated handlers. Paths here are
/// relative to the `/api` scope; the prefix is applied by [`ApiDoc`].
#[derive(OpenApi)]
#[openapi(
    paths(
        tag::get_taglist,
        tag::get_tag,
        tag::get_tag_task,
        submission::get_submission,
        submission::get_submission_job,
        submission::get_submission_search,
        submit_github::github_submission,
        submit_gitlab::gitlab_submit_webhook,
    ),
    components(schemas(
        ErrorResponse,
        SubmitResponse,
        TagListResponse,
        TagResponse,
        SubmissionResponse,
        SubmissionJobWithReportResponse,
    ))
)]
struct ApiEndpoints;

/// Declares the security schemes referenced by name in the handler annotations.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use crate::auth::COOKIE_NAME_API_AUTH_KEY;

        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "api_token",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(format!(
                        "API token, sent either as `Authorization: Bearer <token>` or in the \
                         `{COOKIE_NAME_API_AUTH_KEY}` cookie. A request is authenticated if \
                         either one carries a valid token, so the cookie also works for links \
                         opened straight from a browser."
                    )))
                    .build(),
            ),
        );
        components.add_security_scheme(
            "github_webhook",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("X-Hub-Signature-256"))),
        );
        components.add_security_scheme(
            "gitlab_webhook",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("X-Gitlab-Token"))),
        );
    }
}

/// The full API document: the endpoints nested under the `/api` scope prefix.
#[derive(OpenApi)]
#[openapi(
    nest((path = "/api", api = ApiEndpoints)),
    modifiers(&SecurityAddon),
    info(
        title = "ID2202 Autograder REST API",
        description = "External interface for querying grading tags, retrieving \
                       submission results, and receiving webhook submissions.",
    ),
)]
pub struct ApiDoc;

/// Serializes the assembled OpenAPI specification as pretty-printed JSON.
pub fn spec_json() -> Result<String, Error> {
    ApiDoc::openapi()
        .to_pretty_json()
        .map_err(|e| Error::runtime(format!("could not serialize OpenAPI spec: {e}")))
}
