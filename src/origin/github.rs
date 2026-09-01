//! Various GitHub related utilities
// Using this library for HTTP: https://docs.rs/reqwest/latest/reqwest/

use crate::{
    config::{
        settings::{GitHubServerSettings, KnownInstance},
        Settings,
    },
    error::Error,
    origin::{OriginKind, SSHGitFetch},
    reporting::MetaReport,
};
use reqwest::{
    self,
    header::{HeaderMap, HeaderValue},
    Client as ReqwestClient,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug)]
pub struct GitHub;

#[derive(Clone, Debug)]
pub struct GitHubInfo {
    pub instance: GitHubServerSettings,
    pub organization_name: String,
    pub repo_name: String,
    pub commit_hash: String,
}

#[derive(Clone, Copy, Debug)]
pub enum CommitState {
    Error,
    Failure,
    Pending,
    Success,
}

impl CommitState {
    fn as_str(&self) -> &'static str {
        match self {
            CommitState::Error => "error",
            CommitState::Failure => "failure",
            CommitState::Pending => "pending",
            CommitState::Success => "success",
        }
    }
}

/// Common HTTP headers for GitHub API calls.
fn common_headers(
    _settings: &Settings,
    instance: &GitHubServerSettings,
) -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
    headers.insert("Accept", HeaderValue::from_static("application/vnd.github+json"));
    headers.insert("X-GitHub-Api-Version", HeaderValue::from_static("2022-11-28"));
    headers.insert(
        "Authorization",
        format!("Bearer {}", instance.auth_token).parse().map_err(|e| {
            log::error!("Could not convert github auth token to header value");
            Error::parse_type(
                "GitHub auth token header value".to_string(),
                instance.auth_token.clone(),
            )
            .with_cause(Box::new(e))
        })?,
    );
    Ok(headers)
}

impl OriginKind for GitHub {
    type Info = GitHubInfo;
    type SubmissionState = CommitState;
    type Fetch = SSHGitFetch;

    fn fetch_spec(_settings: &Settings, info: &Self::Info) -> Self::Fetch {
        SSHGitFetch {
            ssh_url: format!(
                "ssh://{}@{}:{}/{}/{}.git",
                info.instance.ssh_user,
                info.instance.outbound_host(),
                info.instance.ssh_port,
                info.organization_name,
                info.repo_name,
            ),
            commit_hash: info.commit_hash.to_string(),
        }
    }

    /// Creates a commit message for that specific repo and commit hash
    /// https://docs.github.com/en/enterprise-server@3.16/rest/commits/statuses?apiVersion=2022-11-28#create-a-commit-status
    async fn set_state(
        settings: &Settings,
        info: &Self::Info,
        state: &Self::SubmissionState,
        description: Option<&str>,
    ) -> Result<(), Error> {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        struct GhCommitStatus {
            state: String,
            description: Option<String>,
        }

        let headers = common_headers(settings, &info.instance)?;
        let c = ReqwestClient::new();
        let response = c
            .post(format!(
                "https://{}/api/v3/repos/{}/{}/statuses/{}",
                info.instance.outbound_domain(),
                info.organization_name,
                info.repo_name,
                info.commit_hash
            ))
            .headers(headers)
            .json(&GhCommitStatus {
                state: state.as_str().to_string(),
                description: description.map(|s| s.to_owned()),
            })
            .send()
            .await
            .map_err(|e| {
                log::error!("Error with GitHub commit status: {e}");
                e
            })?;

        if response.status().is_success() {
            log::debug!("Successfully created commit status on commit {}", info.commit_hash);
            Ok(())
        } else {
            Error::err_http_response(
                "when creating commit status".to_string(),
                response.status().as_u16(),
                response.text().await.unwrap_or("no text received".to_string()),
            )
        }
    }

    /// Creates a commit message for that specific repo and commit hash,
    /// formatting the provided report as markdown.
    /// https://docs.github.com/en/enterprise-server@3.16/rest/commits/comments?apiVersion=2022-11-28#create-a-commit-comment
    async fn send_report<'a>(
        settings: &Settings,
        info: &Self::Info,
        report: &MetaReport<'a>,
    ) -> Result<(), Error> {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        struct GhCommitMessage {
            body: String,
        }

        let headers = common_headers(settings, &info.instance)?;
        let c = ReqwestClient::new();
        let response = c
            .post(format!(
                "https://{}/api/v3/repos/{}/{}/commits/{}/comments",
                info.instance.outbound_domain(),
                info.organization_name,
                info.repo_name,
                info.commit_hash
            ))
            .headers(headers)
            .json(&GhCommitMessage {
                body: format!(
                    "{}\n\n{}",
                    report.formatter_markdown(&settings.reporting),
                    settings.submission.comment_signature
                ),
            })
            .send()
            .await
            .map_err(|e| {
                log::error!("Error with GitHub commit: {e}");
                Error::auto_msg("error with GitHub commit request", e)
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

// Addtitional functionalities that can be used with GitHub origins

/// Returns `Ok(true)` if the repo exists, `Ok(false)` if it does not exist,
/// and an error if there was something wrong with the request.
pub async fn repo_exists(
    settings: &Settings,
    instance: &GitHubServerSettings,
    organization_name: &str,
    repo_name: &str,
) -> Result<bool, Error> {
    let c = ReqwestClient::new();
    let response = c
        .get(format!(
            "https://{}/api/v3/repos/{}/{}",
            instance.outbound_domain(),
            organization_name,
            repo_name
        ))
        .headers(common_headers(settings, instance)?)
        .send()
        .await
        .map_err(|e| {
            log::error!("Error with getting GitHub repo: {e}");
            e
        })?;

    Ok(response.status().is_success())
}

/// Creates a new blank repository with the specified `repo_name`, in the
/// specified `organization_name`. Returns `Ok(())` on success.
///
/// This should primarily be used to create shadow repositories.
pub async fn create_repo(
    settings: &Settings,
    instance: &GitHubServerSettings,
    organization_name: &str,
    repo_name: &str,
    private: bool,
) -> Result<(), Error> {
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct GhCreateRepo {
        /// Name of the repository to create.
        name: String,

        /// Whether this repository should be private.
        private: bool,
    }

    let c = ReqwestClient::new();
    let response = c
        .post(format!(
            "https://{}/api/v3/orgs/{}/repos",
            instance.outbound_domain(),
            organization_name
        ))
        .headers(common_headers(settings, instance)?)
        .json(&GhCreateRepo { name: repo_name.to_owned(), private })
        .send()
        .await
        .map_err(|e| {
            log::error!("Error with getting GitHub repo: {e}");
            e
        })?;

    if response.status().is_success() {
        Ok(())
    } else {
        Error::err_http_response(
            "when creating GitHub repository".to_string(),
            response.status().as_u16(),
            response.text().await.unwrap_or("no text received".to_string()),
        )
    }
}
