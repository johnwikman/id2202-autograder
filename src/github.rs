// Various GitHub related utilities
// https://docs.rs/reqwest/latest/reqwest/

use crate::{error::Error, settings::Settings};
use reqwest::{
    self,
    header::{HeaderMap, HeaderValue},
    Client as ReqwestClient,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GhCommitMessage {
    body: String,
}

fn common_headers(settings: &Settings) -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Accept",
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        "X-GitHub-Api-Version",
        HeaderValue::from_static("2022-11-28"),
    );
    headers.insert(
        "Authorization",
        format!("Bearer {}", settings.github.auth_token)
            .parse()
            .map_err(|_| {
                log::error!("Could not convert github auth token to header value");
                Error::from("Invalid header value")
            })?,
    );
    Ok(headers)
}

/// Creates a commit message for that specific repo and commit hash
/// https://docs.github.com/en/enterprise-server@3.16/rest/commits/comments?apiVersion=2022-11-28#create-a-commit-comment
pub async fn create_commit_message(
    settings: &Settings,
    organization_name: &str,
    repo_name: &str,
    commit_hash: &str,
    message: &str,
) -> Result<(), Error> {
    let c = ReqwestClient::new();
    let response = c
        .post(format!(
            "https://{}/api/v3/repos/{}/{}/commits/{}/comments",
            settings.github.address, organization_name, repo_name, commit_hash
        ))
        .headers(common_headers(settings)?)
        .json(&GhCommitMessage {
            body: format!("{}\n\n{}", message, settings.github.comment_signature),
        })
        .send()
        .await
        .map_err(|e| {
            log::error!("Error with GitHub commit: {e}");
            e
        })?;

    if 200 <= response.status().as_u16() && response.status().as_u16() < 300 {
        log::debug!("Successfully posted comment to commit {}", commit_hash);
        Ok(())
    } else {
        let errmsg = format!(
            "Non-OK response code {} when submitting commit comment: {}",
            response.status(),
            response
                .text()
                .await
                .unwrap_or("no text received".to_string())
        );
        log::error!("{}", errmsg);
        Err(Error::from(errmsg))
    }
}

pub enum CommitState {
    Error,
    Failure,
    Pending,
    Success,
}
impl CommitState {
    fn to_str(&self) -> &str {
        match self {
            CommitState::Error => "error",
            CommitState::Failure => "failure",
            CommitState::Pending => "pending",
            CommitState::Success => "success",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GhCommitStatus {
    state: String,
    description: Option<String>,
}

/// Creates a commit message for that specific repo and commit hash
/// https://docs.github.com/en/enterprise-server@3.16/rest/commits/statuses?apiVersion=2022-11-28#create-a-commit-status
pub async fn create_commit_status(
    settings: &Settings,
    organization_name: &str,
    repo_name: &str,
    commit_hash: &str,
    state: CommitState,
    description: Option<&str>,
) -> Result<(), Error> {
    let c = ReqwestClient::new();
    let response = c
        .post(format!(
            "https://{}/api/v3/repos/{}/{}/statuses/{}",
            settings.github.address, organization_name, repo_name, commit_hash
        ))
        .headers(common_headers(settings)?)
        .json(&GhCommitStatus {
            state: state.to_str().to_string(),
            description: description.map(|s| s.to_owned()),
        })
        .send()
        .await
        .map_err(|e| {
            log::error!("Error with GitHub commit status: {e}");
            e
        })?;

    if 200 <= response.status().as_u16() && response.status().as_u16() < 300 {
        log::debug!(
            "Successfully created commit status on commit {}",
            commit_hash
        );
        Ok(())
    } else {
        let errmsg = format!(
            "Non-OK response code {} when creating commit status: {}",
            response.status(),
            response
                .text()
                .await
                .unwrap_or("no text received".to_string())
        );
        log::error!("{}", errmsg);
        Err(Error::from(errmsg))
    }
}

/// Returns `Ok(true)` if the repo exists, `Ok(false)` if it does not exist,
/// and an error if there was something wrong with the request.
pub async fn repo_exists(
    settings: &Settings,
    organization_name: &str,
    repo_name: &str,
) -> Result<bool, Error> {
    let c = ReqwestClient::new();
    let response = c
        .get(format!(
            "https://{}/api/v3/repos/{}/{}",
            settings.github.address, organization_name, repo_name
        ))
        .headers(common_headers(settings)?)
        .send()
        .await
        .map_err(|e| {
            log::error!("Error with getting GitHub repo: {e}");
            e
        })?;

    if 200 <= response.status().as_u16() && response.status().as_u16() < 300 {
        Ok(true)
    } else {
        Ok(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GhCreateRepo {
    /// Name of the repository to create.
    name: String,

    /// Whether this repository should be private.
    private: bool,
}

/// Creates a new blank repository with the specified `repo_name`, in the
/// specified `organization_name`. Returns `Ok(())` on success.
///
/// This should primarily be used to create shadow repositories.
pub async fn create_repo(
    settings: &Settings,
    organization_name: &str,
    repo_name: &str,
    private: bool,
) -> Result<(), Error> {
    let c = ReqwestClient::new();
    let response = c
        .post(format!(
            "https://{}/api/v3/orgs/{}/repos",
            settings.github.address, organization_name
        ))
        .headers(common_headers(settings)?)
        .json(&GhCreateRepo {
            name: repo_name.to_owned(),
            private: private,
        })
        .send()
        .await
        .map_err(|e| {
            log::error!("Error with getting GitHub repo: {e}");
            e
        })?;

    if 200 <= response.status().as_u16() && response.status().as_u16() < 300 {
        Ok(())
    } else {
        let errmsg = format!(
            "Non-OK response code {} when creating GitHub repo: {}",
            response.status(),
            response
                .text()
                .await
                .unwrap_or("no text received".to_string())
        );
        log::error!("{}", errmsg);
        Err(Error::from(errmsg))
    }
}
