use std::{collections::BTreeSet, time::Duration};

//use std::ffi::OsString;
use serde::{Deserialize, Serialize};

use crate::{
    error::Error,
    utils::{syscommand_timeout, SyscommandSettings},
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
        &["podman", "images", "--format", "json"],
        SyscommandSettings {
            max_stdout_length: Some(128 * 1024),
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        },
    )?;
    if output.code != 0 {
        return Err(Error::from(format!(
            "Error code {} when reading podman images: {}",
            output.code, output.stderr
        )));
    }

    let imglist: Vec<PodmanImageOutput> = serde_json::from_str(&output.stdout)
        .map_err(|e| Error::from(format!("Could not deserialize podman output: {e}")))?;

    let mut imgs: BTreeSet<String> = BTreeSet::new();
    for img in imglist.iter() {
        for imgname in img.names.iter() {
            imgs.insert(imgname.to_owned());
        }
    }

    Ok(Vec::from_iter(imgs.into_iter()))
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
        &["podman", "network", "list", "--format", "json"],
        SyscommandSettings {
            max_stdout_length: Some(128 * 1024),
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        },
    )?;
    if output.code != 0 {
        return Err(Error::from(format!(
            "Error code {} when reading podman networks: {}",
            output.code, output.stderr
        )));
    }

    let netlist: Vec<PodmanNetworkOutput> = serde_json::from_str(&output.stdout)
        .map_err(|e| Error::from(format!("Could not deserialize podman output: {e}")))?;

    let nets: BTreeSet<String> = BTreeSet::from_iter(netlist.into_iter().map(|pno| pno.name));

    Ok(Vec::from_iter(nets.into_iter()))
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

/// Returns a list of podman networks on the system.
pub fn ps_names() -> Result<Vec<String>, Error> {
    let output = syscommand_timeout(
        &["podman", "ps", "-a", "--format", "json"],
        SyscommandSettings {
            max_stdout_length: Some(128 * 1024),
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        },
    )?;
    if output.code != 0 {
        return Err(Error::from(format!(
            "Error code {} when fetching podman containers: {}",
            output.code, output.stderr
        )));
    }

    let pslist: Vec<PodmanPSOutput> = serde_json::from_str(&output.stdout)
        .map_err(|e| Error::from(format!("Could not deserialize podman output: {e}")))?;

    let names: BTreeSet<String> =
        BTreeSet::from_iter(pslist.into_iter().flat_map(|ppso| ppso.names.into_iter()));

    Ok(Vec::from_iter(names.into_iter()))
}

/// Returns a list of podman networks on the system.
pub fn ps() -> Result<Vec<PodmanPSOutput>, Error> {
    let output = syscommand_timeout(
        &["podman", "ps", "-a", "--format", "json"],
        SyscommandSettings {
            max_stdout_length: Some(128 * 1024),
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        },
    )?;
    if output.code != 0 {
        return Err(Error::from(format!(
            "Error code {} when fetching podman containers: {}",
            output.code, output.stderr
        )));
    }

    let pslist: Vec<PodmanPSOutput> = serde_json::from_str(&output.stdout)
        .map_err(|e| Error::from(format!("Could not deserialize podman output: {e}")))?;

    Ok(pslist)
}

/// Pulls an image. This will time out after 20 minutes, which should be
/// sufficient for even the larger images.
pub fn pull(tag: &str) -> Result<(), Error> {
    let output = syscommand_timeout(
        &["podman", "pull", tag],
        SyscommandSettings {
            timeout: Duration::from_secs(1200),
            ..Default::default()
        },
    )?;
    if output.code != 0 {
        return Err(Error::from(format!(
            "Error code {} when pulling podman image {}",
            output.code, tag
        )));
    }

    Ok(())
}

/// Create a network
pub fn create_network(network_name: &str) -> Result<(), Error> {
    let output = syscommand_timeout(
        &["podman", "network", "create", "--disable-dns", network_name],
        SyscommandSettings {
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        },
    )?;
    if output.code != 0 {
        return Err(Error::from(format!(
            "Error code {} when creating podman network {}: {}",
            output.code, network_name, output.stderr
        )));
    }

    Ok(())
}

pub struct ContainerOptions {
    pub image: String,
    pub container_name: String,
    pub network_name: String,
    pub mounts: Vec<(String, String, String)>,
}

/// Create a container and start it
pub fn start_container(opts: &ContainerOptions) -> Result<(), Error> {
    let mut cmd: Vec<String> = vec![
        "podman".to_string(),
        "run".to_string(),
        "--detach".to_string(),
        "--rm".to_string(),
        "--name".to_string(),
        opts.container_name.to_owned(),
        "--hostname".to_string(),
        opts.container_name.to_owned(),
        "--uts".to_string(),
        "private".to_owned(),
        "--network".to_string(),
        opts.network_name.to_owned(),
    ];
    for (host_path, mount_path, opts) in opts.mounts.iter() {
        cmd.push("-v".to_owned());
        cmd.push(format!("{host_path}:{mount_path}:{opts}"));
    }
    cmd.push(opts.image.to_owned());
    // Set the image to just loop indefinitely
    cmd.push("bash".to_string());
    cmd.push("-c".to_string());
    cmd.push("while true; do sleep 1; done".to_string());

    let output = syscommand_timeout(
        cmd.iter()
            .map(String::as_ref)
            .collect::<Vec<&str>>()
            .as_slice(),
        SyscommandSettings {
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        },
    )?;
    if output.code != 0 {
        log::error!("Error starting container with command: {cmd:?}");
        return Err(Error::from(format!(
            "Error code {} when starting podman container {}: {}",
            output.code, opts.container_name, output.stderr
        )));
    }

    Ok(())
}

/// Execute a command in a running container, without the need to check for any
/// of the return values.
pub fn exec(container_name: &str, exec_cmd: &[&str]) -> Result<(), Error> {
    let mut cmd: Vec<&str> = vec!["podman", "exec", container_name];
    cmd.extend_from_slice(exec_cmd);

    let output = syscommand_timeout(
        cmd.as_slice(),
        SyscommandSettings {
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        },
    )?;
    if output.code != 0 {
        return Err(Error::from(format!(
            "Error code {} when executing command {:?}: {}",
            output.code, cmd, output.stderr
        )));
    }

    Ok(())
}

/// Force removal of a container
pub fn force_rm(container_name: &str) -> Result<(), Error> {
    let output = syscommand_timeout(
        &["podman", "rm", "-f", "-t", "0", container_name],
        SyscommandSettings {
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        },
    )?;
    if output.code != 0 {
        return Err(Error::from(format!(
            "Error code {} when removing podman container {}: {}",
            output.code, container_name, output.stderr
        )));
    }

    Ok(())
}
