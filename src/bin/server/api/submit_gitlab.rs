use std::fmt::Display;

use actix_web::{
    web::{self, Buf},
    HttpRequest, Responder,
};
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use id2202_autograder::{
    config::{settings::GitLabServerSettings, Settings},
    db::conn::DatabaseConnection,
    gitlab,
};

use crate::api::{
    common::{extract_grading_tags, validate_repo_prefix_suffix},
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

    /// Repository name within the namespace
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

/// Convenient struct with the information necessary to create a commit message
/// and a commit status.
#[derive(Debug)]
struct CommitMessageInfo<'a> {
    settings: &'a Settings,
    instance: &'a GitLabServerSettings,
    namespace: &'a str,
    sub: &'a GitLabSubmission,
}

impl<'a> CommitMessageInfo<'a> {
    async fn post_msg_status(
        &self,
        msg: &impl Display,
        status: gitlab::CommitState,
        status_msg: Option<&str>,
    ) -> Result<(), id2202_autograder::error::Error> {
        gitlab::create_commit_message(
            self.settings,
            self.instance,
            self.namespace,
            &self.sub.project.name,
            &self.sub.after,
            msg,
        )
        .await
        .inspect_err(|e| log::error!("Error creating commit message: {e}"))?;

        gitlab::set_commit_status(
            self.settings,
            self.instance,
            self.namespace,
            &self.sub.project.name,
            &self.sub.after,
            status,
            status_msg,
        )
        .await
        .inspect_err(|e| log::error!("Error creating commit status: {e}"))?;

        Ok(())
    }
}

/// Submission from GitLab. From a webhook
///
/// This is just used for testing for now.
pub async fn gitlab_submit_webhook(
    data: web::Data<Settings>,
    req: HttpRequest,
    payload: web::Payload,
) -> Result<impl Responder, actix_web::Error> {
    let settings = data.get_ref();

    log::info!(
        "GitLab submission request from {} (Hook UUID: {})",
        req.peer_addr()
            .map(|addr| addr.to_string())
            .unwrap_or("unknown".to_string()),
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
        return Err(ErrorResponse::unauthorized(&req, "invalid github token").into());
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

    let (namespace, repo_name) = match sub
        .project
        .path_with_namespace
        .split('/')
        .collect::<Vec<&str>>()
        .as_slice()
    {
        &[ns, repo] => (ns, repo),
        _ => {
            return Err(
                ErrorResponse::bad_request(&req, "wrong format on path_with_namespace").into(),
            );
        }
    };
    if repo_name != sub.project.name {
        return Err(
            ErrorResponse::bad_request(&req, "inconsistent repository name in submission").into(),
        );
    }

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
            return Ok(SubmitResponse::new(
                &req,
                "pushed commits do not point to the head of the repository",
            )
            .to_http());
        }
    };

    let commitinfo = CommitMessageInfo {
        settings: &settings,
        instance: &instance_settings,
        namespace: namespace,
        sub: &sub,
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
        return Ok(SubmitResponse::new(&req, "not a repository to be graded").to_http());
    }

    let grading_tags: Vec<&str> = match extract_grading_tags(&settings, &commit_to_grade.message) {
        Ok(tags) => tags,
        Err(rep) => {
            commitinfo
                .post_msg_status(
                    &rep.formatter_markdown(&settings.reporting),
                    gitlab::CommitState::Canceled,
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
            sub.project.path_with_namespace
        );
        return Ok(SubmitResponse::new(&req, "no grading tags provided").to_http());
    }

    // Connect to database and insert the submission request
    let mut dbconn = DatabaseConnection::connect(&settings).map_err(|err| {
        log::error!("Could not connect to database: {err}");
        ErrorResponse::internal_server_error(&req)
    })?;

    let submission_id = dbconn
        .register_gitlab_submission(
            &grading_tags,
            &domain,
            &sub.user_username,
            &namespace,
            &sub.project.name,
            &sub.project.ssh_url,
            &sub.after,
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
        ), gitlab::CommitState::Pending, Some("Waiting In Queue"))
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
