//! Selection of the storage podman operates on, and utilities for running
//! commands on a store.
//!
//! Regular podman commands without an explicit store, like `podman ps -a`,
//! should use the `SHARED` store as `SHARED.podman_cmd(&["ps", "-a"])`.

use std::time::Duration;

use serde::{de::DeserializeOwned, Deserialize};

use crate::{
    error::Error,
    utils::{syscommand_timeout, SyscommandOutput, SyscommandSettings},
};

/// Partially deserialized output from `podman info`.
#[derive(Debug, Clone, Deserialize)]
pub struct Info {
    pub host: InfoHost,
    pub store: InfoStore,
}
#[derive(Debug, Clone, Deserialize)]
pub struct InfoHost {
    #[serde(rename = "cgroupVersion")]
    pub cgroup_version: String,
    #[serde(rename = "cgroupControllers")]
    pub cgroup_controllers: Vec<String>,
    #[serde(rename = "idMappings")]
    pub id_mappings: InfoHostIdMappings,
    #[serde(rename = "networkBackend")]
    pub network_backend: String,
    #[serde(rename = "networkBackendInfo")]
    pub network_backend_info: InfoHostNetworkBackend,
    pub security: InfoHostSecurity,
    pub pasta: InfoHostPasta,
}
#[derive(Debug, Clone, Deserialize)]
pub struct InfoHostIdMappings {
    pub gidmap: Option<Vec<InfoHostIdMap>>,
    pub uidmap: Option<Vec<InfoHostIdMap>>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct InfoHostIdMap {
    pub size: usize,
}
#[derive(Debug, Clone, Deserialize)]
pub struct InfoHostNetworkBackend {
    pub path: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct InfoHostSecurity {
    pub rootless: bool,
}
#[derive(Debug, Clone, Deserialize)]
pub struct InfoHostPasta {
    pub executable: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct InfoStore {
    #[serde(rename = "graphRoot")]
    pub graph_root: String,
}

/// The storage a podman command reads and writes. A command whose `root` no
/// longer exists silently creates a new empty store there.
#[derive(Debug, Default)]
pub struct Store {
    root: Option<String>,
    runroot: Option<String>,
    network_config_dir: Option<String>,
    driver: Option<String>,
    opts: Vec<String>,
    transient: bool,
}

/// The storage podman selects when given no flags.
pub const SHARED: Store = Store {
    root: None,
    runroot: None,
    driver: None,
    network_config_dir: None,
    opts: vec![],
    transient: false,
};

pub const FUSE_OVERLAYFS: &str = "/usr/bin/fuse-overlayfs";

impl Store {
    /// Storage holding its layers under `root` and its runtime state under
    /// `runroot`. `runroot` must not be inside `root`.
    pub fn isolated(root: impl Into<String>, runroot: impl Into<String>) -> Self {
        Self { root: Some(root.into()), runroot: Some(runroot.into()), ..Self::default() }
    }

    /// Sets the storage driver, e.g. `"overlay"`.
    pub fn with_driver(mut self, driver: impl Into<String>) -> Self {
        self.driver = Some(driver.into());
        self
    }

    /// Adds a `--storage-opt` entry, e.g.
    /// `"overlay.mount_program=/usr/bin/fuse-overlayfs"`.
    pub fn with_opt(mut self, opt: impl Into<String>) -> Self {
        self.opts.push(opt.into());
        self
    }

    /// Keeps podman's database under `runroot` instead of under `root`. Every
    /// command on this storage has to use the same setting.
    pub fn with_transient_store(mut self) -> Self {
        self.transient = true;
        self
    }

    /// Keeps network definitions in `dir` instead of under `root`.
    pub fn with_network_config_dir(mut self, dir: impl Into<String>) -> Self {
        self.network_config_dir = Some(dir.into());
        self
    }

    /// The command line up to, but not including, the subcommand.
    fn argv(&self) -> Vec<&str> {
        let mut argv = vec!["podman"];
        if let Some(root) = &self.root {
            argv.extend(["--root", root.as_str()]);
        }
        if let Some(runroot) = &self.runroot {
            argv.extend(["--runroot", runroot.as_str()]);
        }
        if let Some(dir) = &self.network_config_dir {
            argv.extend(["--network-config-dir", dir.as_str()]);
        }
        if let Some(driver) = &self.driver {
            argv.extend(["--storage-driver", driver.as_str()]);
        }
        for opt in self.opts.iter() {
            argv.extend(["--storage-opt", opt.as_str()]);
        }
        if self.transient {
            argv.push("--transient-store");
        }
        argv
    }

    /// Runs the podman CLI with this storage's flags followed by `args`.
    /// Allows custom timeout and syscommand behavior through the `settings`
    /// argument.
    pub fn podman_cmd_with<S: AsRef<str>, ArgList: AsRef<[S]>>(
        &self,
        args: ArgList,
        settings: SyscommandSettings,
    ) -> Result<SyscommandOutput, Error> {
        let mut argv = self.argv();
        argv.extend(args.as_ref().iter().map(|s| s.as_ref()));
        syscommand_timeout(argv, settings)
    }

    /// Runs the podman CLI with this storage's flags followed by `args`. Fails
    /// on a non-zero exit, after 60 seconds, or on more than 1 MiB of stdout
    /// or 128 KiB of stderr.
    pub fn podman_cmd<S: AsRef<str>, ArgList: AsRef<[S]>>(
        &self,
        args: ArgList,
    ) -> Result<SyscommandOutput, Error> {
        self.podman_cmd_with(
            args,
            SyscommandSettings {
                expected_code: Some(0),
                max_stdout_length: Some(1024 * 1024),
                max_stderr_length: Some(128 * 1024),
                timeout: Duration::from_secs(60),
                ..Default::default()
            },
        )
    }

    /// Like [`Store::podman_cmd`], parsing stdout as JSON into `T`.
    pub fn podman_cmd_json<T: DeserializeOwned, S: AsRef<str>, ArgList: AsRef<[S]>>(
        &self,
        args: ArgList,
    ) -> Result<T, Error> {
        let res = self.podman_cmd(&args)?;
        serde_json::from_str(&res.stdout).map_err(|e| {
            Error::convert(format!(
                "could not deserialize JSON output from podman command: {:?}",
                args.as_ref().iter().map(|s| s.as_ref()).collect::<Vec<_>>()
            ))
            .with_cause(e)
        })
    }

    pub fn info(&self) -> Result<Info, Error> {
        self.podman_cmd_json(["info", "--format", "json"])
    }
}

/// The directory podman keeps the shared storage in.
pub fn graph_root() -> Result<String, Error> {
    SHARED.info().map(|i| i.store.graph_root)
}

/// Where network definitions should be stored under the specified
/// `graph_root`.
pub fn network_config_dir(graph_root: &str) -> String {
    format!("{graph_root}/networks")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_store_is_a_bare_podman_invocation() {
        assert_eq!(SHARED.argv(), ["podman"]);
    }

    #[test]
    fn test_isolated_store_argv() {
        let store = Store::isolated("/ws/scratch/storage", "/ws/runroot")
            .with_driver("overlay")
            .with_opt("overlay.mount_program=/usr/bin/fuse-overlayfs")
            .with_opt("additionalimagestore=/var/lib/containers/storage");

        assert_eq!(
            store.argv(),
            [
                "podman",
                "--root",
                "/ws/scratch/storage",
                "--runroot",
                "/ws/runroot",
                "--storage-driver",
                "overlay",
                "--storage-opt",
                "overlay.mount_program=/usr/bin/fuse-overlayfs",
                "--storage-opt",
                "additionalimagestore=/var/lib/containers/storage",
            ]
        );
    }
}
