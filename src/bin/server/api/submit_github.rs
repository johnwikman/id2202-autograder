use std::fmt::Display;

use actix_web::{
    web::{self, Buf},
    HttpRequest, Responder,
};
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use id2202_autograder::{
    config::{settings::GitHubServerSettings, Settings},
    db::conn::DatabaseConnection,
    github,
};

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

use crate::api::{
    common::{extract_grading_tags, validate_repo_prefix_suffix},
    response::{ErrorResponse, SubmitResponse},
};

/// A serializable submission, based on the JSON blob that is provided by the
/// server.
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

    /// URL for cloning the repository over SSH
    ssh_url: String,
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
    instance: &'a GitHubServerSettings,
    sub: &'a GitHubSubmission,
}

impl<'a> CommitMessageInfo<'a> {
    async fn post_msg_status(
        &self,
        msg: &impl Display,
        status: github::CommitState,
        status_msg: Option<&str>,
    ) -> Result<(), id2202_autograder::error::Error> {
        github::create_commit_message(
            self.settings,
            self.instance,
            &self.sub.repository.organization,
            &self.sub.repository.name,
            &self.sub.head_commit.id,
            msg,
        )
        .await
        .inspect_err(|e| log::error!("Error creating commit message: {e}"))?;

        github::create_commit_status(
            self.settings,
            self.instance,
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
pub async fn github_submission(
    data: web::Data<Settings>,
    req: HttpRequest,
    payload: web::Payload,
) -> Result<impl Responder, actix_web::Error> {
    let settings = data.get_ref();

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
        .to_bytes_limited(settings.submission.max_payload)
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
        return Ok(SubmitResponse::new(&req, "ping was authenticated").to_http());
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
        instance: &instance_settings,
        sub: &sub,
    };

    if let Err(rejection) = validate_repo_prefix_suffix(
        &sub.repository.organization,
        &sub.repository.name,
        &instance_settings.allowed_orgs,
        &instance_settings.allowed_repo_prefixes,
        &instance_settings.allowed_repo_suffixes,
        &instance_settings.prohibited_repo_prefixes,
        &instance_settings.prohibited_repo_suffixes,
    ) {
        log::info!(
            "Push from {} will not be considered for grading: {}",
            sub.repository.full_name,
            rejection,
        );
        return Ok(SubmitResponse::new(&req, "not a repository to be graded").to_http());
    }

    let grading_tags: Vec<&str> =
        match extract_grading_tags(&settings, sub.head_commit.message.as_ref()) {
            Ok(tags) => tags,
            Err(rep) => {
                commitinfo
                    .post_msg_status(
                        &rep.formatter_markdown(&settings.reporting),
                        github::CommitState::Failure,
                        Some("Invalid Grading Tags"),
                    )
                    .await
                    .unwrap_or_else(|e| log::warn!("Could not submit commit info: {e}."));

                return Ok(SubmitResponse::new(&req, "bad grading tags").to_http());
            }
        };

    if grading_tags.is_empty() {
        log::info!(
            "Push from {} will not be considered for grading, no grading tags provided",
            sub.repository.full_name
        );
        return Ok(SubmitResponse::new(&req, "no grading tags provided").to_http());
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
            &sub.repository.ssh_url,
            &sub.head_commit.id,
        )
        .map_err(|e| {
            log::error!("Could not register submission with database: {e}");
            ErrorResponse::internal_server_error(&req)
        })?;

    // Respond to the commit message and set the commit status
    commitinfo.post_msg_status(&format!(
        "**[Submission ID: {} | {}]**\n\n{} {}",
        submission_id,
        grading_tags.iter().format_with(", ", |t, f| f(&format_args!("`{t}`"))),
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

    log::info!("Submission {sub:?} successfully inserted with id {submission_id}");
    Ok(SubmitResponse::new(&req, &format!("submission {submission_id} received")).to_http())
}
