//! Containers and the operations that inspect or remove them.

use std::{
    borrow::Cow,
    collections::BTreeSet,
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};

use super::{
    store::Store,
    teardown::{OnDrop, Teardown},
};
use crate::{
    error::Error,
    utils::{SyscommandOutput, SyscommandSettings},
};

/// A selection of JSON output fields when listing the containers
///
/// See:
/// https://docs.podman.io/en/latest/_static/api.html#tag/containers/operation/ContainerListLibpod
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PsOutput {
    #[serde(rename = "AutoRemove")]
    pub auto_remove: bool,

    #[serde(rename = "Names", default)]
    pub names: Vec<String>,

    #[serde(rename = "Exited")]
    pub exited: bool,

    #[serde(rename = "State")]
    pub state: String,

    #[serde(rename = "Status")]
    pub status: String,
}

/// Returns a list of the podman containers in `store`.
pub fn ps(store: &Store) -> Result<Vec<PsOutput>, Error> {
    store.podman_cmd_json(["ps", "-a", "--format", "json"])
}

/// Returns a list of names of the podman containers in `store` (running or
/// exited).
pub fn ps_names(store: &Store) -> Result<Vec<String>, Error> {
    let names = BTreeSet::from_iter(ps(store)?.into_iter().flat_map(|o| o.names.into_iter()));
    Ok(Vec::from_iter(names))
}

/// A selection of JSON output fields when inspecting a container
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InspectOutput {
    #[serde(rename = "Id")]
    pub id: String,

    #[serde(rename = "Name")]
    pub name: String,

    #[serde(rename = "State")]
    pub state: InspectState,
}

/// The container state reported by `podman inspect`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InspectState {
    #[serde(rename = "Status")]
    pub status: String,

    #[serde(rename = "Running")]
    pub running: bool,

    #[serde(rename = "OOMKilled")]
    pub oom_killed: bool,

    #[serde(rename = "ExitCode")]
    pub exit_code: i32,

    #[serde(rename = "CgroupPath", default)]
    pub cgroup_path: String,
}

/// Returns the inspected state of a container in `store`.
pub fn inspect(store: &Store, container_name: &str) -> Result<InspectOutput, Error> {
    let inspected: Vec<InspectOutput> =
        store.podman_cmd_json(["inspect", "--type", "container", container_name])?;

    inspected.into_iter().next().ok_or_else(|| {
        Error::runtime(format!("podman inspect returned nothing for \"{container_name}\""))
    })
}

/// Force removal of a container in `store`.
pub fn force_rm(store: &Store, container_name: &str) -> Result<(), Error> {
    store.podman_cmd(["rm", "-f", "-t", "0", container_name])?;
    Ok(())
}

/// Force-removes every container in `store`, regardless of their execution
/// status.
pub fn rm_all(store: &Store) -> Result<(), Error> {
    store.podman_cmd(["rm", "-f", "-t", "0", "--all"])?;
    Ok(())
}

/// Kills every running container in `store` and waits until all of them have
/// exited. The containers and their storage are not removed.
pub fn kill_all(store: &Store) -> Result<(), Error> {
    store.podman_cmd(["kill", "--all"])?;
    let names = ps_names(store)?;
    if !names.is_empty() {
        let mut args = vec!["wait", "--ignore"];
        args.extend(names.iter().map(|s| s.as_str()));
        store.podman_cmd(args)?;
    }
    Ok(())
}

/// A host directory made available inside a container.
#[derive(Debug, Clone)]
pub struct Mount {
    pub host_path: String,
    pub container_path: String,
    pub writable: bool,
}

/// A podman container that is removed when dropped. Use an `Rc`/`Arc` when
/// necessary to share a container.
pub type Container = OnDrop<WeakContainer>;

impl Container {
    /// Wrapper for [`WeakContainer::new`].
    pub fn new(image: impl Into<String>, name: impl Into<String>, store: Store) -> Self {
        WeakContainer::new(image, name, store).into()
    }
}

/// A podman container. Deliberately does not implement `Clone`.
///
/// # Note
/// The container is left as is when this struct is dropped.
/// (Weak resource reference.)
#[derive(Debug)]
pub struct WeakContainer {
    image: String,
    name: String,

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

    /// Memory limit in podman's notation, e.g. `"256m"`. Swap is capped at the
    /// same value, leaving none.
    pub memory: Option<String>,

    /// Fail instead of pulling an image that is missing from the store.
    pub pull_never: bool,

    /// Let podman remove a started container as soon as it exits. Without
    /// this, an exited container stays inspectable until torn down.
    pub remove_on_exit: bool,

    store: Store,

    start_polling_interval: Duration,

    is_started: bool,
}

impl WeakContainer {
    /// A container in `store`, with no network, no mounts and no limits.
    pub fn new(image: impl Into<String>, name: impl Into<String>, store: Store) -> Self {
        Self {
            image: image.into(),
            name: name.into(),
            network: None,
            mounts: vec![],
            read_only: false,
            drop_privileges: false,
            pids_limit: None,
            memory: None,
            pull_never: false,
            remove_on_exit: true,
            store,
            start_polling_interval: Duration::from_millis(100),
            is_started: false,
        }
    }

    /// The name identifying this container to podman.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The storage this container lives in.
    pub fn store(&self) -> &Store {
        &self.store
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
            // The podman/stable image defaults to sharing these namespaces
            // with its host and to no cgroups, which would otherwise silently
            // drops the limits below.
            "--uts".into(),
            "private".into(),
            "--ipc".into(),
            "private".into(),
            "--cgroupns".into(),
            "private".into(),
            "--cgroups".into(),
            "enabled".into(),
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
            // --memory-swap being the same as --memory means that zero
            // additional space is available for swap
            flags.extend([
                "--memory".into(),
                memory.as_str().into(),
                "--memory-swap".into(),
                memory.as_str().into(),
            ]);
        }
        if self.pull_never {
            flags.push("--pull=never".into());
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
            return Err(Error::runtime(format!("container \"{}\" is already started", self.name)));
        }
        let flags = self.flags();
        let mut args = vec!["run", "--detach"];
        if self.remove_on_exit {
            args.push("--rm");
        }
        args.extend(flags.iter().map(|f| f.as_ref()));
        args.push(&self.image);
        args.extend(["bash", "-c", "while true; do sleep 1; done"]);

        self.store.podman_cmd(&args)?;
        self.is_started = true;

        Ok(())
    }

    /// Blocks until the container has started, or until the time limit has
    /// been exceeded.
    pub fn block_until_started(&self, limit: Duration) -> Result<(), Error> {
        log::debug!("Waiting until \"{}\" is started", self.name);

        let init_time = SystemTime::now();

        loop {
            for ps_output in ps(&self.store)?.iter() {
                if ps_output.names.contains(&self.name) && ps_output.state == "running" {
                    log::debug!("\"{}\" is running", self.name);
                    return Ok(());
                }
            }

            let diff = SystemTime::now().duration_since(init_time)?;

            if diff > limit {
                return Error::err_runtime(format!(
                    "\"{}\" did not start after {} seconds",
                    self.name,
                    limit.as_secs_f64()
                ));
            }

            std::thread::sleep(self.start_polling_interval);
        }
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
        let mut args = vec!["run", "--rm"];
        if settings.stdin.is_some() {
            args.push("-i");
        }
        args.extend(flags.iter().map(|f| f.as_ref()));
        args.push(&self.image);
        args.extend(exec_cmd.as_ref().iter().map(|s| s.as_ref()));

        self.store.podman_cmd_with(&args, settings)
    }

    /// Runs a command in the started container.
    pub fn exec<S: AsRef<str>, CmdList: AsRef<[S]>>(
        &self,
        workdir: Option<&str>,
        exec_cmd: CmdList,
        settings: SyscommandSettings,
    ) -> Result<SyscommandOutput, Error> {
        if !self.is_started {
            return Err(Error::runtime(format!("container \"{}\" is not started", self.name)));
        }
        let mut args = vec!["exec"];
        if let Some(workdir) = workdir {
            args.extend(["-w", workdir]);
        }
        if settings.stdin.is_some() {
            args.push("-i");
        }
        args.push(&self.name);
        args.extend(exec_cmd.as_ref().iter().map(|s| s.as_ref()));

        self.store.podman_cmd_with(&args, settings)
    }

    /// Blocks until the container has exited. Returns immediately if the
    /// container does not exist.
    pub fn wait(&self) -> Result<(), Error> {
        self.store.podman_cmd(["wait", "--ignore", self.name.as_str()])?;
        Ok(())
    }
}

impl Teardown for WeakContainer {
    /// Removes the container and verifies that is was removed.
    fn teardown(&mut self) -> Result<(), Error> {
        if !self.is_started {
            return Ok(());
        }

        force_rm(&self.store, &self.name).map_err(|e| {
            Error::runtime(format!("could not remove the container \"{}\"", self.name))
                .with_cause(e)
        })?;

        if ps_names(&self.store)?.contains(&self.name) {
            log::warn!("container \"{}\" was not successfully removed.", self.name);
        } else {
            self.is_started = false;
        }

        Ok(())
    }
}
