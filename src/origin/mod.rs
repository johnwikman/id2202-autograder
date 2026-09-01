//! Functionality related to different submission origins.

pub mod github;
pub mod gitlab;

use std::future::Future;

use crate::config::Settings;
use crate::error::Error;
use crate::reporting::MetaReport;
use crate::utils::{syscommand_timeout, SyscommandSettings};

/// A submission origin.
#[derive(Debug, Clone)]
pub struct Origin<K: OriginKind> {
    pub info: K::Info,
}

impl<K: OriginKind> Origin<K> {
    /// Convenient way to call K::Fetch::fetch_into(...) on the current info.
    pub fn fetch_into(&self, settings: &Settings, dir: &str) -> Result<(), Error> {
        K::fetch_spec(settings, &self.info).fetch_into(settings, dir)
    }

    /// Sets the state of the submission at the origin.
    pub async fn set_state(
        &self,
        settings: &Settings,
        state: &K::SubmissionState,
        description: Option<&str>,
    ) -> Result<(), Error> {
        K::set_state(settings, &self.info, state, description).await
    }

    /// Sends a report to the submission origin.
    pub async fn send_report(
        &self,
        settings: &Settings,
        report: &MetaReport<'_>,
    ) -> Result<(), Error> {
        K::send_report(settings, &self.info, report).await
    }

    /// Sets the state of the submission at the origin and sends a report to
    /// the origin as well. Will perform both actions concurrently, and
    /// report an error if any of them fails.
    pub async fn set_state_and_report(
        &self,
        settings: &Settings,
        report: &MetaReport<'_>,
        state: &K::SubmissionState,
        state_description: Option<&str>,
    ) -> Result<(), Error> {
        let future_state = self.set_state(settings, state, state_description);
        let future_report = self.send_report(settings, report);

        match tokio::join!(future_state, future_report) {
            (Ok(_), Ok(_)) => Ok(()),
            (Ok(_), Err(e_rep)) => Err(e_rep),
            (Err(e_state), Ok(_)) => Err(e_state),
            (Err(e_state), Err(e_rep)) => Error::err_multi_cause(
                "could not set state nor send report",
                vec![e_state.into(), e_rep.into()],
            ),
        }
    }
}

/// Common information that an origin needs to provide.
pub trait OriginKind {
    type Info;
    type SubmissionState;
    type Fetch: FetchSpec;

    /// Returns a spec for how to fetch the submitted code
    fn fetch_spec(settings: &Settings, info: &Self::Info) -> Self::Fetch;

    /// Sets the state of the submission at the origin.
    fn set_state(
        settings: &Settings,
        info: &Self::Info,
        state: &Self::SubmissionState,
        description: Option<&str>,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Sends a report to the submission origin.
    fn send_report(
        settings: &Settings,
        info: &Self::Info,
        report: &MetaReport,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}

pub trait FetchSpec {
    /// Fetches the submitted solution, populating it under the provided
    /// directory `dir`.
    fn fetch_into(&self, settings: &Settings, dir: &str) -> Result<(), Error>;
}

/// Customized fetch functionality for git-based origins over SSH.
pub struct SSHGitFetch {
    pub ssh_url: String,
    pub commit_hash: String,
}

impl FetchSpec for SSHGitFetch {
    fn fetch_into(&self, settings: &Settings, dir: &str) -> Result<(), Error> {
        let gitcmd_settings = SyscommandSettings { expected_code: Some(0), ..Default::default() };
        let quote = |p: &str| {
            shlex::try_quote(p).map(String::from).map_err(|e| {
                Error::auto_msg(format!("Could not quote {p} for the SSH command: {e}"), e)
            })
        };
        let mut ssh_cmd = format!(
            "core.sshCommand=ssh -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile={}",
            quote(&settings.runner.ssh_known_hosts)?,
        );
        if !settings.runner.ssh_keys.is_empty() {
            ssh_cmd.push_str(" -o IdentitiesOnly=yes");
            for key in &settings.runner.ssh_keys {
                ssh_cmd.push_str(&format!(" -i {}", quote(key)?));
            }
        }

        // A way to check out a specific commit, without the cloning the whole
        // history of the repository.
        std::fs::create_dir_all(dir)
            .map_err(Error::from)
            .and_then(|_| {
                syscommand_timeout(["git", "-C", dir, "init"], gitcmd_settings.to_owned())
            })
            .and_then(|_| {
                syscommand_timeout(
                    ["git", "-C", dir, "remote", "add", "origin", &self.ssh_url],
                    gitcmd_settings.to_owned(),
                )
            })
            .and_then(|_| {
                syscommand_timeout(
                    [
                        "git",
                        "-C",
                        dir,
                        "-c",
                        &ssh_cmd,
                        "fetch",
                        "--depth",
                        "1",
                        "origin",
                        &self.commit_hash,
                    ],
                    gitcmd_settings.to_owned(),
                )
            })
            .and_then(|_| {
                syscommand_timeout(
                    ["git", "-C", dir, "checkout", "FETCH_HEAD"],
                    gitcmd_settings.to_owned(),
                )
            })?;

        Ok(())
    }
}
