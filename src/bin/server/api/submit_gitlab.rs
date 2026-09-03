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
        models::{NewSubmissionOriginGitLabRow, SubmissionStatus},
    },
    origin::{
        gitlab::{self, GitLab, GitLabInfo},
        Origin,
    },
    reporting::MetaReport,
};

use crate::api::{
    common::{
        acceptance_message, extract_grading_tags, internal_error_report, report_superseded,
        resolve_jobs, validate_repo_prefix_suffix,
    },
    response::{ErrorResponse, SubmitResponse},
};

/// A serializable GitLab submission, based on the JSON blob that is provided
/// by the server.
#[derive(Debug, Serialize, Deserialize)]
struct GitLabSubmission {
    before: String,
    after: String,
    user_username: String,
    project: GlsProject,
    commits: Vec<GlsCommit>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GlsProject {
    /// Full repository name (format: `{NAMESPACE}/{REPO}`)
    path_with_namespace: String,

    /// Written out repository display name.
    name: String,

    /// The URL to access the website
    web_url: String,

    /// The URL to access repo over SSH
    ssh_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GlsCommit {
    id: String,
    message: String,
    timestamp: String,
}

/// Submission from GitLab. From a webhook
///
/// This is just used for testing for now.
#[utoipa::path(
    tag = "Submissions",
    params(
        ("X-Gitlab-Event" = String, Header, description = "Event type, e.g. `Push Hook`"),
        ("X-Gitlab-Token" = String, Header, description = "GitLab webhook authentication token"),
        ("X-Gitlab-Webhook-UUID" = String, Header, description = "Unique identifier of the webhook delivery."),
    ),
    security(("gitlab_webhook" = [])),
    responses(
        (status = 200, description = "Webhook was accepted, but no submission was registered.", body = SubmitResponse),
        (status = 201, description = "Submission created and registered in the database.", body = SubmitResponse),
        (status = 400, description = "Malformed webhook payload.", body = ErrorResponse),
        (status = 401, description = "Invalid webhook token.", body = ErrorResponse),
    ),
)]
#[post("/submit/gitlab")]
pub async fn gitlab_submit_webhook(
    data: web::Data<Settings>,
    req: HttpRequest,
    payload: web::Payload,
) -> Result<impl Responder, actix_web::Error> {
    let settings = data.get_ref();

    log::info!(
        "GitLab submission request from {} (Hook UUID: {})",
        req.peer_addr().map(|addr| addr.to_string()).unwrap_or("unknown".to_string()),
        req.headers()
            .get("X-Gitlab-Webhook-UUID")
            .and_then(|hv| hv.to_str().ok())
            .unwrap_or("unknown"),
    );

    // Disregard it request immediately if it is not a GitLab event
    let gl_event = req
        .headers()
        .get("X-Gitlab-Event")
        .and_then(|hv| hv.to_str().ok())
        .ok_or_else(|| ErrorResponse::bad_request(&req, "missing event type"))?;

    // Validate the submission request
    let gl_token = req
        .headers()
        .get("X-Gitlab-Token")
        .and_then(|hv| hv.to_str().ok())
        .ok_or_else(|| ErrorResponse::unauthorized(&req, "missing gitlab token"))?;
    if gl_token != settings.submission.gitlab.webhook_secret {
        return Err(ErrorResponse::unauthorized(&req, "invalid gitlab token").into());
    }

    log::debug!("Submission request authorized.");

    if gl_event.trim().to_lowercase() != "push hook" {
        log::warn!("Received invalid event type {}", gl_event);
        return Err(ErrorResponse::bad_request(
            &req,
            &format!("invalid event type \"{gl_event}\""),
        )
        .into());
    }

    // Decode the payload as JSON
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

    // Decode the payload as JSON
    let sub: GitLabSubmission = serde_json::from_slice(payload_bytes.chunk()).map_err(|err| {
        log::warn!("Received invalid JSON payload: {err:?}");
        ErrorResponse::bad_request(&req, "invalid JSON format")
    })?;

    log::debug!("Received push event: {:?}", sub);

    // Fetch the domain of the submission, verify that we have it configured as
    // a source
    let parsed_url = url::Url::parse(&sub.project.web_url).map_err(|err| {
        log::warn!("Received invalid repository URL: {err}");
        ErrorResponse::bad_request(&req, "Invalid repository URL")
    })?;

    let domain = parsed_url
        .domain()
        .map(|dom| match parsed_url.port() {
            Some(p) => format!("{dom}:{p}"),
            None => dom.to_string(),
        })
        .ok_or_else(|| {
            log::warn!("Received submission without domain in the repository URL");
            ErrorResponse::bad_request(&req, "Invalid repository URL")
        })?;

    let (namespace, repo_name) =
        match sub.project.path_with_namespace.split('/').collect::<Vec<&str>>().as_slice() {
            &[ns, repo] => (ns, repo),
            _ => {
                return Err(ErrorResponse::bad_request(
                    &req,
                    "wrong format on path_with_namespace",
                )
                .into());
            }
        };

    let instance_settings = settings
        .submission
        .gitlab
        .known_instances
        .iter()
        .find(|gl| gl.domain == domain)
        .ok_or_else(|| {
            log::warn!("Received request from unknown GitLab instance {domain}");
            ErrorResponse::unauthorized(&req, "Unknown GitLab instance")
        })?;

    // Extract the commit information pointing to the head of the repository
    let commit_to_grade = match sub.commits.iter().find(|c| c.id == sub.after) {
        Some(c) => c,
        None => {
            return Ok(SubmitResponse::without_id(
                &req,
                "pushed commits do not point to the head of the repository",
            )
            .to_http());
        }
    };

    let origin = Origin::<GitLab> {
        info: GitLabInfo {
            instance: instance_settings.clone(),
            namespace: namespace.to_string(),
            repo_name: repo_name.to_string(),
            commit_hash: sub.after.to_string(),
        },
    };

    if let Err(rejection) = validate_repo_prefix_suffix(
        namespace,
        repo_name,
        &instance_settings.allowed_namespaces,
        &instance_settings.allowed_repo_prefixes,
        &instance_settings.allowed_repo_suffixes,
        &instance_settings.prohibited_repo_prefixes,
        &instance_settings.prohibited_repo_suffixes,
    ) {
        log::info!(
            "Push from {} will not be considered for grading: {}",
            sub.project.path_with_namespace,
            rejection,
        );
        return Ok(SubmitResponse::without_id(&req, "not a repository to be graded").to_http());
    }

    let grading_tags: Vec<&str> = match extract_grading_tags(settings, &commit_to_grade.message) {
        Ok(tags) => tags,
        Err(rep) => {
            origin
                .set_state_and_report(
                    settings,
                    &rep.as_ref().into(),
                    &gitlab::CommitState::Canceled,
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
            sub.project.path_with_namespace
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
                // Fatal by design: a test configuration that will not load means
                // the runner cannot grade anything either, so accepting the
                // submission would only hide it.
                log::error!("Could not load test configuration: {e}");
                None
            }
        };
    let jobs = match &tests {
        Some(tests) => resolve_jobs(tests, &grading_tags),
        None => Err(Box::new(internal_error_report())),
    };

    let source = NewSubmissionOriginGitLabRow {
        domain: domain.clone(),
        namespace: namespace.to_string(),
        repo: repo_name.to_string(),
        ssh_url: sub.project.ssh_url.clone(),
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
                .register_ungradable_submission::<GitLab>(
                    &grading_tags,
                    &report,
                    &source,
                    &sub.user_username,
                    &sub.after,
                )
                .map_err(|e| {
                    log::error!("Could not register submission with database: {e}");
                    ErrorResponse::internal_server_error(&req)
                })?;

            origin
                .set_state_and_report(
                    settings,
                    &report.as_ref().into(),
                    &gitlab::CommitState::Failed,
                    Some("Submission Error"),
                )
                .await
                .unwrap_or_else(|e| log::warn!("Could not submit commit info: {e}."));

            log::info!("Submission {} recorded, but nothing can be graded", submission.id);
            return Ok(
                SubmitResponse::new(&req, "submission cannot be graded", submission.id).to_http()
            );
        }
    };

    let registered = dbconn
        .register_submission::<GitLab>(&grading_tags, jobs, &source, &sub.user_username, &sub.after)
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
        SubmissionStatus::Aborted => (gitlab::CommitState::Canceled, "Nothing To Grade"),
        _ => (gitlab::CommitState::Pending, "Waiting In Queue"),
    };

    // Respond to the commit message and set the commit status
    origin
        .set_state_and_report(
            settings,
            &MetaReport::Structured(acceptance_message(&registered.submission)),
            &state,
            Some(label),
        )
        .await
        .unwrap_or_else(|e| log::warn!("Could not submit commit info: {e}. Will not reject this submission since it is already created."));

    // Notifying the other runners (TODO: make this name configurable)
    dbconn.notify("submission").unwrap_or_else(|e| {
        log::warn!("Could not notify the runners about the new submission: {}", e)
    });

    log::info!("Submission {sub:?} successfully inserted with id {submission_id}");
    Ok(SubmitResponse::new(&req, "submission received", submission_id).to_http())
}
