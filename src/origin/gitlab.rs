//! Various GitLab related utilities

use std::collections::BTreeMap;

use crate::{
    config::{
        settings::{GitLabServerSettings, KnownInstance},
        Settings,
    },
    error::Error,
    origin::{OriginKind, SSHGitFetch},
    reporting::MetaReport,
};
use reqwest::{self, header::HeaderMap, Client as ReqwestClient};

#[derive(Clone, Copy, Debug)]
pub struct GitLab;

#[derive(Clone, Debug)]
pub struct GitLabInfo {
    pub instance: GitLabServerSettings,
    pub namespace: String,
    pub repo_name: String,
    pub commit_hash: String,
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
    fn as_str(self) -> &'static str {
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

/// Common headers used for all GitLab requests.
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

impl OriginKind for GitLab {
    type Info = GitLabInfo;
    type SubmissionState = CommitState;
    type Fetch = SSHGitFetch;

    fn fetch_spec(_settings: &Settings, info: &Self::Info) -> Self::Fetch {
        SSHGitFetch {
            ssh_url: format!(
                "ssh://{}@{}:{}/{}/{}.git",
                info.instance.ssh_user,
                info.instance.outbound_host(),
                info.instance.ssh_port,
                info.namespace,
                info.repo_name,
            ),
            commit_hash: info.commit_hash.to_string(),
        }
    }

    /// Sets a GitLab commit status
    /// https://docs.gitlab.com/api/commits/#set-commit-pipeline-status
    async fn set_state(
        settings: &Settings,
        info: &Self::Info,
        state: &Self::SubmissionState,
        description: Option<&str>,
    ) -> Result<(), Error> {
        let c = ReqwestClient::new();

        let headers = common_headers(settings, &info.instance)?;
        let mut form_params = BTreeMap::new();
        form_params.insert("state", state.as_str());
        if let Some(desc) = description {
            form_params.insert("description", desc);
        }

        let response = c
            .post(format!(
                "{}://{}/api/v4/projects/{}/statuses/{}",
                if info.instance.use_https { "https" } else { "http" },
                info.instance.outbound_domain(),
                repo_id(&info.namespace, &info.repo_name),
                info.commit_hash
            ))
            .headers(headers)
            .form(&form_params)
            .send()
            .await
            .map_err(|e| {
                log::error!("Error with GitLab commit: {e}");
                Error::auto_msg("error with GitLab commit request", e)
            })?;

        if response.status().is_success() {
            log::debug!("Successfully posted status to commit {}", info.commit_hash);
            Ok(())
        } else {
            Error::err_http_response(
                "when submitting commit status".to_string(),
                response.status().as_u16(),
                response.text().await.unwrap_or("no text received".to_string()),
            )
        }
    }

    /// Creates a commit message for that specific repo and commit hash
    /// https://docs.gitlab.com/api/commits/#post-comment-to-commit
    async fn send_report<'a>(
        settings: &Settings,
        info: &Self::Info,
        report: &MetaReport<'a>,
    ) -> Result<(), Error> {
        let c = ReqwestClient::new();

        let headers = common_headers(settings, &info.instance)?;
        let mut form_params = BTreeMap::new();
        form_params.insert(
            "note",
            format!(
                "{}\n\n{}",
                report.formatter_markdown(&settings.reporting),
                settings.submission.comment_signature
            ),
        );

        let response = c
            .post(format!(
                "{}://{}/api/v4/projects/{}/repository/commits/{}/comments",
                if info.instance.use_https { "https" } else { "http" },
                info.instance.outbound_domain(),
                repo_id(&info.namespace, &info.repo_name),
                info.commit_hash
            ))
            .headers(headers)
            .form(&form_params)
            .send()
            .await
            .map_err(|e| {
                log::error!("Error with GitLab commit: {e}");
                Error::auto_msg("error with GitLab commit request", e)
            })?;

        if response.status().is_success() {
            log::debug!("Successfully posted comment to commit {}", info.commit_hash);
            Ok(())
        } else {
            Error::err_http_response(
                "when submitting commit comment".to_string(),
                response.status().as_u16(),
                response.text().await.unwrap_or("no text received".to_string()),
            )
        }
    }
}
