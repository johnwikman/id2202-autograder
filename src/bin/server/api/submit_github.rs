use actix_web::{
    post,
    web::{self, Buf},
    HttpRequest, Responder,
};
use serde::{Deserialize, Serialize};

use id2202_autograder::{
    config::{Settings, Tests, TestsLoadingOptions},
    db::{
        conn::DatabaseConnection,
        models::{NewSubmissionOriginGitHubRow, SubmissionStatus},
    },
    origin::{
        github::{self, GitHub, GitHubInfo},
        Origin,
    },
    reporting::MetaReport,
};

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

use crate::api::{
    common::{
        acceptance_message, extract_grading_tags, internal_error_report, report_superseded,
        resolve_jobs, validate_repo_prefix_suffix,
    },
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

/// Submission from GitHub. Received a webhook
///
/// See documentation over at docs.github.com/enterprise-server@3.16/webhooks/
#[utoipa::path(
    tag = "Submissions",
    params(
        ("X-Github-Event" = String, Header, description = "Event type: `push` or `ping`"),
        ("X-Github-Hook-ID" = String, Header, description = "Unique identifier of the webhook"),
        ("X-Hub-Signature-256" = String, Header, description = "Hashed authentication of the webhook, on the format `sha256=<lower case hex>`."),
    ),
    security(("github_webhook" = [])),
    responses(
        (status = 200, description = "Webhook was accepted, but no submission was registered.", body = SubmitResponse),
        (status = 201, description = "Submission created and registered in the database.", body = SubmitResponse),
        (status = 400, description = "Malformed webhook payload.", body = ErrorResponse),
        (status = 401, description = "Invalid webhook signature.", body = ErrorResponse),
    ),
)]
#[post("/submit/github")]
pub async fn github_submission(
    data: web::Data<Settings>,
    req: HttpRequest,
    payload: web::Payload,
) -> Result<impl Responder, actix_web::Error> {
    let settings = data.get_ref();

    log::info!(
        "GitHub submission request from {} (Hook ID: {})",
        req.peer_addr().map(|addr| addr.to_string()).unwrap_or("unknown".to_string()),
        req.headers().get("X-Github-Hook-ID").and_then(|hv| hv.to_str().ok()).unwrap_or("unknown"),
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
        .ok_or(ErrorResponse::unauthorized(&req, "missing secret signature"))?
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
    let hmac256_computed = format!("sha256={}", hex::encode(mac_output_vec));

    log::debug!("Computed hash: \"{}\"", hmac256_computed);
    log::debug!("Received hash: \"{}\"", hmac256_received);

    if hmac256_received != hmac256_computed {
        log::warn!("Unauthorized submission request.");
        return Err(ErrorResponse::unauthorized(&req, "invalid secret signature").into());
    }

    log::debug!("Submission request authorized.");

    // Validate the github event
    if gh_event == "ping" {
        return Ok(SubmitResponse::without_id(&req, "ping was authenticated").to_http());
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

    let origin = Origin::<GitHub> {
        info: GitHubInfo {
            instance: instance_settings.clone(),
            organization_name: sub.repository.organization.clone(),
            repo_name: sub.repository.name.clone(),
            commit_hash: sub.head_commit.id.clone(),
        },
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
        return Ok(SubmitResponse::without_id(&req, "not a repository to be graded").to_http());
    }

    let grading_tags: Vec<&str> =
        match extract_grading_tags(settings, sub.head_commit.message.as_ref()) {
            Ok(tags) => tags,
            Err(rep) => {
                origin
                    .set_state_and_report(
                        settings,
                        &MetaReport::Transient(rep.as_ref()),
                        &github::CommitState::Failure,
                        Some("Invalid Grading Tags"),
                    )
                    .await
                    .unwrap_or_else(|e| log::warn!("Could not submit commit info: {e}."));

                return Ok(SubmitResponse::without_id(&req, "bad grading tags").to_http());
            }
        };

    if grading_tags.is_empty() {
        log::info!(
            "Push from {} will not be considered for grading, no grading tags provided",
            sub.repository.full_name
        );
        return Ok(SubmitResponse::without_id(&req, "no grading tags provided").to_http());
    }

    // Resolve the tags before inserting anything to the database. An unknown
    // tag will result in an immediate error here.
    let tests =
        match Tests::load(&settings.runner.test_config, TestsLoadingOptions { taginfo_only: true })
        {
            Ok(tests) => Some(tests),
            Err(e) => {
                log::error!("FATAL: could not load test configuration: {e}");
                None
            }
        };
    let jobs = match &tests {
        Some(tests) => resolve_jobs(tests, &grading_tags),
        None => Err(Box::new(internal_error_report())),
    };

    let source = NewSubmissionOriginGitHubRow {
        domain: domain.clone(),
        org: sub.repository.organization.clone(),
        repo: sub.repository.name.clone(),
        ssh_url: sub.repository.ssh_url.clone(),
    };

    // Connect to database and insert the submission request
    let mut dbconn = DatabaseConnection::connect(settings).map_err(|err| {
        log::error!("Could not connect to database: {err}");
        ErrorResponse::internal_server_error(&req)
    })?;

    let jobs = match jobs {
        Ok(jobs) => jobs,
        Err(report) => {
            let submission = dbconn
                .register_ungradable_submission::<GitHub>(
                    &grading_tags,
                    &report,
                    &source,
                    &sub.pusher.name,
                    &sub.head_commit.id,
                )
                .map_err(|e| {
                    log::error!("Could not register submission with database: {e}");
                    ErrorResponse::internal_server_error(&req)
                })?;

            origin
                .set_state_and_report(
                    settings,
                    &MetaReport::Transient(&report),
                    &github::CommitState::Failure,
                    Some("Submission Error"),
                )
                .await
                .unwrap_or_else(|e| log::warn!("Could not submit commit info: {e}"));

            log::info!("Submission {} recorded, but nothing can be graded", submission.id);
            return Ok(
                SubmitResponse::new(&req, "submission cannot be graded", submission.id).to_http()
            );
        }
    };

    let registered = dbconn
        .register_submission::<GitHub>(
            &grading_tags,
            jobs,
            &source,
            &sub.pusher.name,
            &sub.head_commit.id,
        )
        .map_err(|e| {
            log::error!("Could not register submission with database: {e}");
            ErrorResponse::internal_server_error(&req)
        })?;
    let submission_id = registered.submission.id;

    report_superseded(settings, &registered)
        .await
        .unwrap_or_else(|e| log::warn!("Could not report superseded jobs: {e}"));

    // A submission whose every tag was rejected has no job left for a runner
    // to pick up, so nothing else would ever move its commit off pending.
    let (state, label) = match registered.submission.status() {
        SubmissionStatus::Aborted => (github::CommitState::Error, "Nothing To Grade"),
        _ => (github::CommitState::Pending, "Waiting In Queue"),
    };

    // Respond to the commit message and set the commit status
    origin.set_state_and_report(settings, &MetaReport::Structured(acceptance_message(&registered.submission)), &state, Some(label))
        .await
        .unwrap_or_else(|e| log::warn!("Could not submit commit info: {e}. Will not reject this submission since it is already created."));

    // Notifying the other runners (TODO: make this name configurable)
    dbconn.notify("submission").unwrap_or_else(|e| {
        log::warn!("Could not notify the runners about the new submission: {}", e)
    });

    log::info!("Submission {sub:?} successfully inserted with id {submission_id}");
    Ok(SubmitResponse::new(&req, "submission received", submission_id).to_http())
}
