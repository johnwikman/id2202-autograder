use std::{borrow::Cow, collections::BTreeSet, time::Duration};

//use std::ffi::OsString;
use serde::{Deserialize, Serialize};

use crate::{
    error::Error,
    utils::{syscommand_timeout, SyscommandOutput, SyscommandSettings},
};

fn default_empty_vec() -> Vec<String> {
    vec![]
}

/// A selection of JSON output fields when listing the images
#[derive(Serialize, Deserialize, Debug, Clone)]
struct PodmanImageOutput {
    #[serde(rename = "Id")]
    pub id: String,

    #[serde(rename = "ParentId")]
    pub parent_id: String,

    #[serde(rename = "RepoDigests")]
    pub repo_digests: Vec<String>,

    #[serde(rename = "Size")]
    pub size: usize,

    #[serde(rename = "Digest")]
    pub digest: String,

    #[serde(rename = "History", default = "default_empty_vec")]
    pub history: Vec<String>,

    #[serde(rename = "Names", default = "default_empty_vec")]
    pub names: Vec<String>,
}

/// Returns a list of podman images on the system.
///
/// This has been tested using Podman 5.6.1
pub fn images() -> Result<Vec<String>, Error> {
    let output = syscommand_timeout(
        ["podman", "images", "--format", "json"],
        SyscommandSettings {
            expected_code: Some(0),
            max_stdout_length: Some(128 * 1024),
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        },
    )?;

    let imglist: Vec<PodmanImageOutput> = serde_json::from_str(&output.stdout)
        .map_err(|e| Error::auto_msg("could not deserialize podman images output", e))?;

    let mut imgs: BTreeSet<String> = BTreeSet::new();
    for img in imglist.iter() {
        for imgname in img.names.iter() {
            imgs.insert(imgname.to_owned());
        }
    }

    Ok(Vec::from_iter(imgs))
}

/// A selection of JSON output fields when listing the networks
#[derive(Serialize, Deserialize, Debug, Clone)]
struct PodmanNetworkOutput {
    pub name: String,
    pub id: String,
    pub driver: String,
    pub network_interface: String,
    pub created: String,
    pub ipv6_enabled: bool,
    pub internal: bool,
    pub dns_enabled: bool,
}

/// Returns a list of podman networks on the system.
pub fn networks() -> Result<Vec<String>, Error> {
    let output = syscommand_timeout(
        ["podman", "network", "list", "--format", "json"],
        SyscommandSettings {
            expected_code: Some(0),
            max_stdout_length: Some(128 * 1024),
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        },
    )?;

    let netlist: Vec<PodmanNetworkOutput> = serde_json::from_str(&output.stdout)
        .map_err(|e| Error::auto_msg("could not deserialize podman network output", e))?;

    let nets: BTreeSet<String> = BTreeSet::from_iter(netlist.into_iter().map(|pno| pno.name));

    Ok(Vec::from_iter(nets))
}

/// A selection of JSON output fields when listing the containers
///
/// See:
/// https://docs.podman.io/en/latest/_static/api.html#tag/containers/operation/ContainerListLibpod
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PodmanPSOutput {
    #[serde(rename = "AutoRemove")]
    pub auto_remove: bool,

    #[serde(rename = "Names", default = "default_empty_vec")]
    pub names: Vec<String>,

    #[serde(rename = "Exited")]
    pub exited: bool,

    #[serde(rename = "State")]
    pub state: String,

    #[serde(rename = "Status")]
    pub status: String,
}

/// Returns a list of names of the running podman containers on the system.
pub fn ps_names() -> Result<Vec<String>, Error> {
    let output = syscommand_timeout(
        ["podman", "ps", "-a", "--format", "json"],
        SyscommandSettings {
            expected_code: Some(0),
            max_stdout_length: Some(128 * 1024),
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        },
    )?;

    let pslist: Vec<PodmanPSOutput> = serde_json::from_str(&output.stdout)
        .map_err(|e| Error::auto_msg("could not deserialize podman ps output", e))?;

    let names: BTreeSet<String> =
        BTreeSet::from_iter(pslist.into_iter().flat_map(|ppso| ppso.names.into_iter()));

    Ok(Vec::from_iter(names))
}

/// Returns a list of podman networks on the system.
pub fn ps() -> Result<Vec<PodmanPSOutput>, Error> {
    let output = syscommand_timeout(
        ["podman", "ps", "-a", "--format", "json"],
        SyscommandSettings {
            expected_code: Some(0),
            max_stdout_length: Some(128 * 1024),
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        },
    )?;

    let pslist: Vec<PodmanPSOutput> = serde_json::from_str(&output.stdout)
        .map_err(|e| Error::auto_msg("could not deserialize podman ps output", e))?;

    Ok(pslist)
}

/// Pulls an image. This will time out after 20 minutes, which should be
/// sufficient for even the larger images.
pub fn pull(tag: &str) -> Result<(), Error> {
    let _output = syscommand_timeout(
        ["podman", "pull", tag],
        SyscommandSettings {
            expected_code: Some(0),
            timeout: Duration::from_secs(1200),
            ..Default::default()
        },
    )?;

    Ok(())
}

/// Builds an image from `context_dir` and tags it `tag`. Shares the pull
/// timeout, since a build starts by pulling its base image. The output is
/// bounded rather than unlimited so that a failed build carries its log in the
/// returned error.
pub fn build(tag: &str, context_dir: &str) -> Result<(), Error> {
    let _output = syscommand_timeout(
        ["podman", "build", "-t", tag, context_dir],
        SyscommandSettings {
            expected_code: Some(0),
            timeout: Duration::from_secs(1200),
            ..Default::default()
        },
    )?;

    Ok(())
}

/// Create a network
pub fn create_network(network_name: &str) -> Result<(), Error> {
    let _output = syscommand_timeout(
        ["podman", "network", "create", "--disable-dns", network_name],
        SyscommandSettings {
            expected_code: Some(0),
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        },
    )?;

    Ok(())
}

/// Force removal of a container
pub fn force_rm(container_name: &str) -> Result<(), Error> {
    let _output = syscommand_timeout(
        ["podman", "rm", "-f", "-t", "0", container_name],
        SyscommandSettings {
            expected_code: Some(0),
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        },
    )?;

    Ok(())
}

/// A host directory made available inside a container.
#[derive(Debug, Clone)]
pub struct Mount {
    pub host_path: String,
    pub container_path: String,
    pub writable: bool,
}

/// A podman container. Deliberately not `Clone`: it is removed when dropped, so
/// share it behind an `Rc`/`Arc` rather than copying it.
#[derive(Debug)]
pub struct PodmanContainer {
    pub image: String,
    pub name: String,

    /// Network to attach to. `None` detaches the container from networking
    /// entirely.
    pub network: Option<String>,

    pub mounts: Vec<Mount>,

    /// Mount the container's own filesystem read-only.
    pub read_only: bool,

    /// Drop every capability and forbid regaining privileges.
    pub drop_privileges: bool,

    /// Upper bound on the number of processes in the container.
    pub pids_limit: Option<u32>,

    /// Memory limit in podman's notation, e.g. `"256m"`.
    pub memory: Option<String>,

    is_started: bool,
}

impl PodmanContainer {
    /// A container with no network, no mounts and no limits. `Default` cannot
    /// be used to fill in the rest of a struct literal, since a type that
    /// implements `Drop` cannot be moved out of.
    pub fn new(image: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            image: image.into(),
            name: name.into(),
            network: None,
            mounts: vec![],
            read_only: false,
            drop_privileges: false,
            pids_limit: None,
            memory: None,
            is_started: false,
        }
    }

    pub fn is_started(&self) -> bool {
        self.is_started
    }

    /// The `podman run` flags describing this container.
    fn flags(&self) -> Vec<Cow<'_, str>> {
        let mut flags: Vec<Cow<'_, str>> = vec![
            "--name".into(),
            self.name.as_str().into(),
            "--hostname".into(),
            self.name.as_str().into(),
            "--uts".into(),
            "private".into(),
            "--network".into(),
            self.network.as_deref().unwrap_or("none").into(),
        ];
        if self.read_only {
            flags.push("--read-only".into());
        }
        if self.drop_privileges {
            flags.extend([
                "--cap-drop".into(),
                "ALL".into(),
                "--security-opt".into(),
                "no-new-privileges".into(),
            ]);
        }
        if let Some(limit) = self.pids_limit {
            flags.extend(["--pids-limit".into(), limit.to_string().into()]);
        }
        if let Some(memory) = &self.memory {
            flags.extend(["--memory".into(), memory.as_str().into()]);
        }
        for mount in self.mounts.iter() {
            let access = if mount.writable { "rw" } else { "ro" };
            flags.push("-v".into());
            flags.push(format!("{}:{}:{access},z", mount.host_path, mount.container_path).into());
        }
        flags
    }

    /// Starts the container in the background, idling until something calls
    /// [`Self::exec`].
    pub fn start(&mut self) -> Result<(), Error> {
        if self.is_started {
            return Err(Error::runtime(format!(
                "container \"{}\" is already started",
                self.name
            )));
        }
        let flags = self.flags();
        let mut cmd: Vec<&str> = vec!["podman", "run", "--detach", "--rm"];
        cmd.extend(flags.iter().map(|f| f.as_ref()));
        cmd.push(&self.image);
        cmd.extend(["bash", "-c", "while true; do sleep 1; done"]);

        syscommand_timeout(
            cmd,
            SyscommandSettings {
                expected_code: Some(0),
                max_stderr_length: Some(128 * 1024),
                ..Default::default()
            },
        )?;
        self.is_started = true;

        Ok(())
    }

    /// Runs a command in a fresh container that is removed afterwards, without
    /// starting this one.
    pub fn run<S: AsRef<str>, CmdList: AsRef<[S]>>(
        &self,
        exec_cmd: CmdList,
        settings: SyscommandSettings,
    ) -> Result<SyscommandOutput, Error> {
        if self.is_started {
            return Err(Error::runtime(format!(
                "container \"{}\" is already started, use exec instead",
                self.name
            )));
        }
        let flags = self.flags();
        let mut cmd: Vec<&str> = vec!["podman", "run", "--rm"];
        if settings.stdin.is_some() {
            cmd.push("-i");
        }
        cmd.extend(flags.iter().map(|f| f.as_ref()));
        cmd.push(&self.image);
        cmd.extend(exec_cmd.as_ref().iter().map(|s| s.as_ref()));

        syscommand_timeout(cmd.as_slice(), settings)
    }

    /// Runs a command in the started container.
    pub fn exec<S: AsRef<str>, CmdList: AsRef<[S]>>(
        &self,
        workdir: Option<&str>,
        exec_cmd: CmdList,
        settings: SyscommandSettings,
    ) -> Result<SyscommandOutput, Error> {
        if !self.is_started {
            return Err(Error::runtime(format!(
                "container \"{}\" is not started",
                self.name
            )));
        }
        let mut cmd: Vec<&str> = vec!["podman", "exec"];
        if let Some(workdir) = workdir {
            cmd.extend(["-w", workdir]);
        }
        if settings.stdin.is_some() {
            cmd.push("-i");
        }
        cmd.push(&self.name);
        cmd.extend(exec_cmd.as_ref().iter().map(|s| s.as_ref()));

        syscommand_timeout(cmd.as_slice(), settings)
    }

    /// Removes the container. Also called on drop.
    pub fn stop(&mut self) {
        if !self.is_started {
            return;
        }
        if let Err(e) = force_rm(&self.name) {
            log::warn!("Could not remove the container \"{}\": {e}", self.name);
        }
        match ps_names() {
            Ok(v) => {
                if v.contains(&self.name) {
                    log::warn!("Container \"{}\" was not removed.", self.name);
                } else {
                    self.is_started = false;
                }
            }
            Err(e) => {
                log::warn!("Could not check for running images: {e}");
            }
        }
    }
}

impl Drop for PodmanContainer {
    fn drop(&mut self) {
        self.stop();
    }
}
