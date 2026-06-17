use std::collections::BTreeMap;

/// Various GitLab related utilities
use crate::{
    config::{settings::GitLabServerSettings, Settings},
    error::Error,
};
use reqwest::{self, header::HeaderMap, Client as ReqwestClient};

fn common_headers(
    _settings: &Settings,
    instance: &GitLabServerSettings,
) -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "PRIVATE-TOKEN",
        instance.auth_token.parse().map_err(|e| {
            log::error!("Could not convert GitLab auth token to header value");
            Error::parse_type(
                "GitLab auth token header value".to_string(),
                instance.auth_token.clone(),
            )
            .with_cause(Box::new(e))
        })?,
    );
    Ok(headers)
}

/// Returns a URL encoded string to be used as the id for GitLab API requests.
fn repo_id(namespace: &str, repo_name: &str) -> String {
    url::form_urlencoded::byte_serialize(format!("{namespace}/{repo_name}").as_bytes()).collect()
}

/// Creates a commit message for that specific repo and commit hash
/// https://docs.gitlab.com/api/commits/#post-comment-to-commit
pub async fn create_commit_message(
    settings: &Settings,
    instance: &GitLabServerSettings,
    namespace: &str,
    repo_name: &str,
    commit_hash: &str,
    message: &impl std::fmt::Display,
) -> Result<(), Error> {
    let c = ReqwestClient::new();

    let mut form_params = BTreeMap::new();
    form_params.insert(
        "note",
        format!("{}\n\n{}", message, settings.submission.comment_signature),
    );

    let response = c
        .post(format!(
            "{}://{}/api/v4/projects/{}/repository/commits/{}/comments",
            if instance.use_https { "https" } else { "http" },
            &instance.domain,
            repo_id(namespace, repo_name),
            commit_hash
        ))
        .headers(common_headers(settings, instance)?)
        .form(&form_params)
        .send()
        .await
        .map_err(|e| {
            log::error!("Error with GitLab commit: {e}");
            Error::auto_msg("error with GitLab commit request", e)
        })?;

    if 200 <= response.status().as_u16() && response.status().as_u16() < 300 {
        log::debug!("Successfully posted comment to commit {}", commit_hash);
        Ok(())
    } else {
        Error::err_http_response(
            "when submitting commit comment".to_string(),
            response.status().as_u16(),
            response
                .text()
                .await
                .unwrap_or("no text received".to_string()),
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CommitState {
    Pending,
    Running,
    Success,
    Failed,
    Canceled,
    Skipped,
}
impl CommitState {
    fn to_str(&self) -> &str {
        match self {
            CommitState::Pending => "pending",
            CommitState::Running => "running",
            CommitState::Success => "success",
            CommitState::Failed => "failed",
            CommitState::Canceled => "canceled",
            CommitState::Skipped => "skipped",
        }
    }
}

/// Sets a GitLab commit status
/// https://docs.gitlab.com/api/commits/#set-commit-pipeline-status
pub async fn set_commit_status(
    settings: &Settings,
    instance: &GitLabServerSettings,
    namespace: &str,
    repo_name: &str,
    commit_hash: &str,
    state: CommitState,
    description: Option<&str>,
) -> Result<(), Error> {
    let c = ReqwestClient::new();

    let mut form_params = BTreeMap::new();
    form_params.insert("state", state.to_str());
    if let Some(desc) = description {
        form_params.insert("description", desc);
    }

    let response = c
        .post(format!(
            "{}://{}/api/v4/projects/{}/statuses/{}",
            if instance.use_https { "https" } else { "http" },
            &instance.domain,
            repo_id(namespace, repo_name),
            commit_hash
        ))
        .headers(common_headers(settings, instance)?)
        .form(&form_params)
        .send()
        .await
        .map_err(|e| {
            log::error!("Error with GitLab commit: {e}");
            Error::auto_msg("error with GitLab commit request", e)
        })?;

    if 200 <= response.status().as_u16() && response.status().as_u16() < 300 {
        log::debug!("Successfully posted status to commit {}", commit_hash);
        Ok(())
    } else {
        Error::err_http_response(
            "when submitting commit status".to_string(),
            response.status().as_u16(),
            response
                .text()
                .await
                .unwrap_or("no text received".to_string()),
        )
    }
}
