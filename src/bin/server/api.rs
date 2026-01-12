use std::collections::BTreeSet;

use actix_web::{
    http::StatusCode,
    web::{self, Buf, ServiceConfig},
    HttpRequest, HttpResponse, Responder, ResponseError,
};
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use id2202_autograder::{
    config::tag_match, config::Settings, db::conn::DatabaseConnection, github,
};

use derive_more::derive::{Display, Error};

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Configuration for the API services.
pub fn config(cfg: &mut ServiceConfig, settings: &Settings) {
    let s = settings.clone();
    cfg.route(
        "/api/github-submit",
        web::post().to(move |req, pl| github_submission(s.clone(), req, pl)),
    );
}

#[derive(Serialize, Debug, Display, Error)]
#[display("error response: {status} on {path}")]
struct ErrorResponse {
    status: u16,
    error: String,
    message: String,
    path: String,
}

impl actix_web::error::ResponseError for ErrorResponse {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).json(self)
    }
    fn status_code(&self) -> actix_web::http::StatusCode {
        StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl ErrorResponse {
    fn unauthorized(req: &HttpRequest, msg: &str) -> ErrorResponse {
        ErrorResponse {
            status: StatusCode::UNAUTHORIZED.as_u16(),
            error: "unauthorized".to_string(),
            message: msg.to_string(),
            path: req.path().to_string(),
        }
    }
    fn bad_request(req: &HttpRequest, msg: &str) -> ErrorResponse {
        ErrorResponse {
            status: StatusCode::BAD_REQUEST.as_u16(),
            error: "bad request".to_string(),
            message: msg.to_string(),
            path: req.path().to_string(),
        }
    }
    fn not_found(req: &HttpRequest, msg: &str) -> ErrorResponse {
        ErrorResponse {
            status: StatusCode::NOT_FOUND.as_u16(),
            error: "not found".to_string(),
            message: msg.to_string(),
            path: req.path().to_string(),
        }
    }
    fn internal_server_error(req: &HttpRequest) -> ErrorResponse {
        ErrorResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
            error: "internal server error".to_string(),
            message: "contact autograder responsible".to_string(),
            path: req.path().to_string(),
        }
    }
}

/// "404: Not found" response for API requests.
/// This function is configured in main.rs
pub fn not_found(req: HttpRequest) -> Result<HttpResponse, actix_web::Error> {
    Ok(ErrorResponse::not_found(&req, "API resource could not be found").error_response())
}

#[derive(Debug, Serialize)]
struct GitHubResponse {
    code: u16,
    message: String,
    path: String,
}

impl GitHubResponse {
    fn new(req: &HttpRequest, msg: &str) -> GitHubResponse {
        GitHubResponse {
            code: StatusCode::OK.as_u16(),
            message: msg.to_string(),
            path: req.path().to_string(),
        }
    }
    fn to_http(&self) -> HttpResponse {
        HttpResponse::Ok().json(self)
    }
}

/// A serializable submission, based on the JSON blob that can be provided from
/// the server.
///
/// https://docs.github.com/en/enterprise-server@3.16/webhooks/webhook-events-and-payloads#push
#[derive(Debug, Serialize, Deserialize)]
struct GitHubSubmission {
    repository: GhsRepository,
    head_commit: GhsHeadCommit,
    pusher: GhsPusher,
}

#[derive(Debug, Serialize, Deserialize)]
struct GhsRepository {
    /// Full repository name (format: `{ORG}/{REPO}`)
    full_name: String,

    /// Repository name within the organization
    name: String,

    /// Organization name
    organization: String,

    /// The base URL to be used for any API calls
    ///
    /// Expected format: `https://{DOMAIN}/api/v3/repos/{ORG}/{REPO}`
    url: String,
}
#[derive(Debug, Serialize, Deserialize)]
struct GhsHeadCommit {
    id: String,
    message: String,
}
#[derive(Debug, Serialize, Deserialize)]
struct GhsPusher {
    name: String,
    email: String,
}

/// Convenient struct with the information necessary to create a commit message
/// and a commit status.
#[derive(Debug)]
struct CommitMessageInfo<'a> {
    settings: &'a Settings,
    domain: &'a str,
    sub: &'a GitHubSubmission,
}

impl<'a> CommitMessageInfo<'a> {
    async fn post_msg_status<S: AsRef<str>>(
        &self,
        msg: S,
        status: github::CommitState,
        status_msg: Option<&str>,
    ) -> Result<(), id2202_autograder::error::Error> {
        github::create_commit_message(
            self.settings,
            self.domain,
            &self.sub.repository.organization,
            &self.sub.repository.name,
            &self.sub.head_commit.id,
            msg.as_ref(),
        )
        .await
        .inspect_err(|e| log::error!("Error creating commit message: {e}"))?;

        github::create_commit_status(
            self.settings,
            self.domain,
            &self.sub.repository.organization,
            &self.sub.repository.name,
            &self.sub.head_commit.id,
            status,
            status_msg,
        )
        .await
        .inspect_err(|e| log::error!("Error creating commit status: {e}"))?;

        Ok(())
    }
}

/// Submission from GitHub. Received a webhook
///
/// See documentation over at docs.github.com/enterprise-server@3.16/webhooks/
///
/// We expect the following for headers:
///   X-Github-Hook-ID:    <id>
///   X-Github-Event:      push | ping
///   X-Hub-Signature-256: sha265=<lower case hex>
async fn github_submission(
    settings: Settings,
    req: HttpRequest,
    payload: web::Payload,
) -> Result<impl Responder, actix_web::Error> {
    log::info!(
        "GitHub submission request from {} (Hook ID: {})",
        req.peer_addr()
            .map(|addr| addr.to_string())
            .unwrap_or("unknown".to_string()),
        req.headers()
            .get("X-Github-Hook-ID")
            .and_then(|hv| hv.to_str().ok())
            .unwrap_or("unknown"),
    );

    // Disregard it request immediately if it is not a GitHub event
    let gh_event = req
        .headers()
        .get("X-Github-Event")
        .and_then(|hv| hv.to_str().ok())
        .ok_or(ErrorResponse::bad_request(&req, "missing event type"))?;

    // Validating the payload with HMAC"
    // See: https://docs.github.com/en/enterprise-server@3.16/webhooks/using-webhooks/validating-webhook-deliveries
    let hmac256_received = req
        .headers()
        .get("X-Hub-Signature-256")
        .and_then(|hv| hv.to_str().ok())
        .ok_or(ErrorResponse::unauthorized(
            &req,
            "missing secret signature",
        ))?
        .to_string();

    let payload_bytes = payload
        .to_bytes_limited(settings.submission.github.max_payload)
        .await
        .map_err(|e| {
            log::warn!("Error reading payload: {e}");
            ErrorResponse::bad_request(&req, "bad payload")
        })?
        .map_err(|e| {
            log::warn!("Error reading payload: {e}");
            ErrorResponse::bad_request(&req, "bad payload")
        })?;

    let mut mac = HmacSha256::new_from_slice(settings.submission.github.webhook_secret.as_bytes())
        .map_err(|hmac_err| {
            log::error!("Could not create HMAC: {hmac_err:?}");
            ErrorResponse::internal_server_error(&req)
        })?;
    mac.update(payload_bytes.chunk());

    let mac_output_vec = mac.finalize().into_bytes();
    let hmac256_computed = format!("sha256={:x}", mac_output_vec);

    log::debug!("Computed hash: \"{}\"", hmac256_computed);
    log::debug!("Received hash: \"{}\"", hmac256_received);

    if hmac256_received != hmac256_computed {
        log::warn!("Unauthorized submission request.");
        return Err(ErrorResponse::unauthorized(&req, "invalid secret signature").into());
    }

    log::debug!("Submission request authorized.");

    // Validate the github event
    if gh_event == "ping" {
        return Ok(GitHubResponse::new(&req, "ping was authenticated").to_http());
    }

    // We only care about push events after this point
    if gh_event != "push" {
        log::warn!("Received invalid event type {}", gh_event);
        return Err(ErrorResponse::bad_request(
            &req,
            &format!("invalid event type \"{gh_event}\""),
        )
        .into());
    }

    // Decode the payload as JSON
    let sub: GitHubSubmission = serde_json::from_slice(payload_bytes.chunk()).map_err(|err| {
        log::warn!("Received invalid JSON payload: {err:?}");
        ErrorResponse::bad_request(&req, "invalid JSON format")
    })?;

    log::debug!("Received push event: {:?}", sub);

    // Fetch the domain of the submission, verify that we have it configured as
    // a source
    let domain = reqwest::Url::parse(&sub.repository.url)
        .map_err(|err| {
            log::warn!("Received invalid repository URL: {err}");
            ErrorResponse::bad_request(&req, "Invalid repository URL")
        })?
        .domain()
        .map(String::from)
        .ok_or_else(|| {
            log::warn!("Received submission without domain in the repository URL");
            ErrorResponse::bad_request(&req, "Invalid repository URL")
        })?;

    let instance_settings = settings
        .submission
        .github
        .known_instances
        .iter()
        .find(|gh| gh.domain == domain)
        .ok_or_else(|| {
            log::warn!("Received request from unknown GitHub instance {domain}");
            ErrorResponse::unauthorized(&req, "Unknown GitHub instance")
        })?;

    let commitinfo = CommitMessageInfo {
        settings: &settings,
        domain: &domain,
        sub: &sub,
    };

    // Check if the repository belongs to a known organization
    if instance_settings.allowed_orgs.len() > 0 {
        if !instance_settings
            .allowed_orgs
            .iter()
            .any(|org| *org == sub.repository.organization)
        {
            let errmsg = format!(
                "invalid GitHub organization \"{}\"",
                sub.repository.organization
            );
            log::warn!("{}", errmsg);
            return Err(ErrorResponse::unauthorized(&req, &errmsg).into());
        }
    }

    // Check for allowed prefixes and rejected suffixes
    if instance_settings.allowed_repo_prefixes.len() > 0 {
        let allowed_prefix = instance_settings
            .allowed_repo_prefixes
            .iter()
            .any(|pfx| sub.repository.name.starts_with(pfx));
        if !allowed_prefix {
            log::info!(
                "Push from {} will not be considered for grading, missing proper prefix",
                sub.repository.full_name
            );
            return Ok(GitHubResponse::new(&req, "not a repository to be graded").to_http());
        }
    }
    if instance_settings.allowed_repo_suffixes.len() > 0 {
        let allowed_suffix = instance_settings
            .allowed_repo_suffixes
            .iter()
            .any(|sfx| sub.repository.name.ends_with(sfx));
        if !allowed_suffix {
            log::info!(
                "Push from {} will not be considered for grading, missing proper suffix",
                sub.repository.full_name
            );
            return Ok(GitHubResponse::new(&req, "not a repository to be graded").to_http());
        }
    }
    let rejected_prefix = instance_settings
        .prohibited_repo_prefixes
        .iter()
        .any(|pfx| sub.repository.name.starts_with(pfx));
    if rejected_prefix {
        log::info!(
            "Push from {} will not be considered for grading, start with invalid prefix",
            sub.repository.full_name
        );
        return Ok(GitHubResponse::new(&req, "not a repository to be graded").to_http());
    }
    let rejected_suffix = instance_settings
        .prohibited_repo_suffixes
        .iter()
        .any(|sfx| sub.repository.name.ends_with(sfx));
    if rejected_suffix {
        log::info!(
            "Push from {} will not be considered for grading, start with invalid suffix",
            sub.repository.full_name
        );
        return Ok(GitHubResponse::new(&req, "not a repository to be graded").to_http());
    }

    // Check for grading tags. First adding them to BTreeSet to remove any
    // duplicates unique, then converting the set back to a vector.
    let mut grading_tag_set: BTreeSet<&str> = BTreeSet::new();
    let mut s: &str = sub.head_commit.message.as_ref();
    while s.len() > 0 {
        // We split at i + 1 because we are interested in the string that
        // follows the tag symbol.
        if let Some((_, s_after)) = s
            .find(|c: char| c == '#' || c == '%')
            .and_then(|i| s.split_at_checked(i + 1))
        {
            let (s_tag, s_rest) = tag_match(s_after);
            grading_tag_set.insert(s_tag);
            s = s_rest;
        } else {
            break;
        }
    }
    let grading_tags: Vec<&str> = grading_tag_set
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();

    if grading_tags.is_empty() {
        log::info!(
            "Push from {} will not be considered for grading, no grading tags provided",
            sub.repository.full_name
        );
        return Ok(GitHubResponse::new(&req, "no grading tags provided").to_http());
    }

    let tag_length = grading_tags
        .iter()
        .map(|s| s.len())
        .reduce(|acc, e| acc + e)
        .unwrap_or(0usize);
    if tag_length >= settings.submission.max_tag_length {
        // Respond to the commit message and set the commit status
        commitinfo.post_msg_status(format!(
            "## Submission Error\n\nThe provided grading tags {} exceed the limit of {} characters. Your submission will not be graded.",
            grading_tags.iter().map(|s| format!("`{}`", s)).join(", "),
            settings.submission.max_tag_length,
        ), github::CommitState::Failure, Some("Tag Length Exceeded"))
        .await
        .unwrap_or_else(|e| log::warn!("Could not submit commit info: {e}."));

        return Ok(GitHubResponse::new(
            &req,
            &format!(
                "maximum tag length exceeded (got {}, limit is {})",
                tag_length, settings.submission.max_tag_length
            ),
        )
        .to_http());
    }

    // Connect to database and insert the submission request
    let mut dbconn = DatabaseConnection::connect(&settings).map_err(|err| {
        log::error!("Could not connect to database: {err}");
        ErrorResponse::internal_server_error(&req)
    })?;

    let submission_id = dbconn
        .register_github_submission(
            &grading_tags,
            &domain,
            &sub.pusher.name,
            &sub.repository.organization,
            &sub.repository.name,
            &sub.head_commit.id,
        )
        .map_err(|e| {
            log::error!("Could not register submission with database: {e}");
            ErrorResponse::internal_server_error(&req)
        })?;

    // Respond to the commit message and set the commit status
    commitinfo.post_msg_status(format!(
        "**[Submission ID: {} | {}]**\n\n{} {}",
        submission_id,
        grading_tags.iter().map(|t| format!("`{t}`")).join(", "),
        "The autograder has successfully received your submission and will start grading as soon as a runner is available.",
        "Additional information and results of your submission will be provided as comments here."
    ), github::CommitState::Pending, Some("Waiting In Queue"))
    .await
    .unwrap_or_else(|e| log::warn!("Could not submit commit info: {e}. Will not reject this submission since it is already created."));

    // Notifying the other runners (TODO: make this name configurable)
    dbconn.notify("submission").unwrap_or_else(|e| {
        log::warn!(
            "Could not notify the runners about the new submission: {}",
            e
        )
    });
    //notify::ping(&settings).unwrap_or_else(|e| {
    //    log::warn!("Could not ping the runners: {}", e);
    //});

    log::info!("Submission {sub:?} successfully inserted with id {submission_id}");
    Ok(GitHubResponse::new(&req, &format!("submission {submission_id} received")).to_http())
}
