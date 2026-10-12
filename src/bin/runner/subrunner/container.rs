use std::{fs::File, time::Duration};

use id2202_autograder::{
    config::types::ByteSize,
    error::Error,
    podman,
    utils::{
        path_absolute, path_absolute_join, syscommand_timeout, SyscommandOutput, SyscommandSettings,
    },
};

/// The directory a runner's grading containers make their scratch filesystem
/// in, and the image store that they read images from.
///
/// # Note
/// This should only be created by `GradingContainer::sweep`.
#[derive(Debug, Clone)]
pub struct ScratchSlot {
    dir: String,
    image_store: String,
}

impl ScratchSlot {
    /// Checking whether the scratch directory exists, returning an `Err` if
    /// it does. This is likely due to a grading container still running.
    pub fn check_available(&self) -> Result<(), Error> {
        if std::fs::exists(&self.dir)? {
            return Err(Error::runtime(format!(
                "the scratch directory \"{}\" is still in use",
                self.dir
            )));
        }
        Ok(())
    }
}

/// A fixed-size podman storage on the filesystem, holding everything its
/// containers write. Container images are read from the graph root image
/// store, shared with all stores and treated as read-only.
///
/// Dropping this struct kills every container inside, removes all networks,
/// and frees up space on the filesystem.
#[derive(Debug)]
pub struct ScratchStore {
    /// The directory holding the ext4 image file, the mount point of the
    /// image, and the runroot.
    dir: String,

    /// The ext4 image file backing the mount.
    backing_file: String,

    /// Where the `backing_file` is mounted.
    mount_point: String,

    /// Podman's runtime state, outside the mount.
    runroot: String,

    /// Podman's storage, inside the mount.
    storage: String,

    /// A directory inside the mount, outside the storage, holding `RESERVE`
    /// bytes and `RESERVE_INODES` inodes.
    reserve: String,

    /// The image store to read container images from.
    image_store: String,
}

impl ScratchStore {
    /// Bytes that have to remain free for the scratch filesystem to count as
    /// usable. A full ext4 keeps reporting up to a megabyte available that it
    /// cannot allocate, whatever size the writes are.
    const DISK_FULL_MARGIN: usize = 4 * 1024 * 1024;

    /// Bytes set aside on the scratch filesystem for removing its containers and
    /// network, which podman cannot do on a filesystem with no space left.
    const RESERVE: usize = 4 * 1024 * 1024;

    /// Inodes set aside on the scratch filesystem for the same purpose as
    /// `RESERVE`.
    const RESERVE_INODES: usize = 32;

    /// Claims ownership of the scratch directory in `slot`, such that dropping
    /// the result also remove the claimed scratch filesystem.
    fn claim(slot: &ScratchSlot) -> Result<ScratchStore, Error> {
        let dir = path_absolute(&slot.dir)?;
        Ok(ScratchStore {
            backing_file: path_absolute_join(&dir, "backing_file")?,
            mount_point: path_absolute_join(&dir, "mnt")?,
            runroot: path_absolute_join(&dir, "runroot")?,
            storage: path_absolute_join(&dir, "mnt/storage")?,
            reserve: path_absolute_join(&dir, "mnt/reserve")?,
            dir,
            image_store: slot.image_store.clone(),
        })
    }

    /// Creates the directory in `slot` and mounts a filesystem of `size` in
    /// it. Fails if the directory in `slot` already exists.
    pub fn create(slot: &ScratchSlot, size: ByteSize) -> Result<ScratchStore, Error> {
        // Note: This will error if slot.dir already exists.
        std::fs::create_dir(&slot.dir).map_err(|e| {
            Error::fs("could not create scratch directory", &slot.dir).with_cause(e)
        })?;
        let store = Self::claim(slot)?;

        // Any early return from here on drops `store`, which unmounts and
        // removes whatever has been set up so far.
        std::fs::create_dir_all(&store.mount_point)?;
        std::fs::create_dir_all(&store.runroot)?;

        let checked = SyscommandSettings {
            expected_code: Some(0),
            max_stderr_length: Some(128 * 1024),
            ..Default::default()
        };
        // Sized so that the reserve does not come out of `size`.
        File::create(&store.backing_file)?.set_len((size.bytes() + Self::RESERVE) as u64)?;

        #[rustfmt::skip]
        let mkfs_cmd = [
            "mkfs.ext4",
            "-q",  // quiet
            "-F",  // force, target is not block device
            "-m", "0", // reserve 0% blocks for root
            "-O", "^has_journal", // no journaling
            "-E", "lazy_itable_init=1,nodiscard", // skip inode table init and discard commands
            &store.backing_file,
        ];
        syscommand_timeout(mkfs_cmd, checked.clone())?;

        // `-o fakeroot` is added as a precaution, but likely not necessary.
        syscommand_timeout(
            ["fuse2fs", &store.backing_file, &store.mount_point, "-o", "fakeroot"],
            checked,
        )?;
        std::fs::create_dir_all(&store.storage)?;
        std::fs::create_dir_all(&store.reserve)?;
        std::fs::write(path_absolute_join(&store.reserve, "space")?, vec![0u8; Self::RESERVE])?;
        for i in 0..Self::RESERVE_INODES {
            std::fs::write(path_absolute_join(&store.reserve, format!("inode{i}"))?, "")?;
        }

        Ok(store)
    }

    /// Opens a previous scratch store if it exists (by a previous unexpected
    /// exit or unhandled failure), returning `Ok(Some(store))` if it exists,
    /// or `Ok(None)` if it does not exist.
    fn open_if_exists(slot: &ScratchSlot) -> Result<Option<ScratchStore>, Error> {
        if !std::fs::exists(&slot.dir)? {
            return Ok(None);
        }
        Self::claim(slot).map(Some)
    }

    /// The podman storage within the scratch store.
    pub fn store(&self) -> podman::Store {
        // See containers-storage.conf(5) for the `.with_opt`s.
        podman::Store::isolated(&self.storage, &self.runroot)
            .with_driver("overlay")
            .with_opt(format!("overlay.mount_program={}", podman::FUSE_OVERLAYFS))
            .with_opt(format!("additionalimagestore={}", self.image_store))
            .with_network_config_dir(podman::network_config_dir(&self.image_store))
            .with_transient_store()
    }

    /// The owner that marks a network as belonging to storage.
    fn network_owner(&self) -> &str {
        &self.dir
    }

    /// Size of the filesystem in bytes, not counting the reserve. Containers
    /// using [`Self::store`] can write at most this much.
    pub fn size(&self) -> Result<usize, Error> {
        let len = std::fs::metadata(&self.backing_file)?.len() as usize;
        Ok(len.saturating_sub(Self::RESERVE))
    }

    /// Whether the scratch store has run out of usable space.
    pub fn is_full(&self) -> Result<bool, Error> {
        // `--output=avail,iavail` shows only two columns: free bytes (due to
        // `-B1`) and free inodes.
        let output = syscommand_timeout(
            ["df", "-B1", "--output=avail,iavail", &self.mount_point],
            SyscommandSettings {
                expected_code: Some(0),
                max_stdout_length: Some(1024),
                max_stderr_length: Some(128 * 1024),
                ..Default::default()
            },
        )?;
        let line = output.stdout.lines().nth(1).unwrap_or_default();
        let fields: Vec<usize> =
            line.split_whitespace().map(str::parse).collect::<Result<_, _>>().map_err(|_| {
                Error::runtime(format!("could not read free space from df: `{line}`"))
            })?;
        match fields[..] {
            [avail, iavail] => Ok(avail < Self::DISK_FULL_MARGIN || iavail == 0),
            _ => Err(Error::runtime(format!("could not read free space from df: \"{line}\""))),
        }
    }
}

impl Drop for ScratchStore {
    fn drop(&mut self) {
        let store = self.store();
        if let Err(e) = podman::kill_all(&store) {
            log::error!("could not kill the containers in store \"{}\": {e}", self.dir);
            return;
        }
        // Now that all containers in store are killed can we safely remove the
        // reserve (extra needed in case of a full filesystem in order to run
        // certain podman commands).
        if let Err(e) = std::fs::remove_dir_all(&self.reserve) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("could not release the reserve \"{}\": {e}", self.reserve);
            }
        }
        if let Err(e) = podman::rm_all(&store) {
            log::warn!("could not remove the containers in \"{}\": {e}", self.dir);
        }
        if let Err(e) = podman::prune_networks(&store, self.network_owner()) {
            log::warn!("could not remove the networks of \"{}\": {e}", self.dir);
        }
        // Unmount any dir under the scratch store and remove the scratch
        // filesystem.
        mounts_under(&self.dir)
            .and_then(|mounts| {
                for mount in mounts {
                    unmount(&mount)
                        .unwrap_or_else(|e| log::error!("could not unmount {mount}: {e}"));
                }
                std::fs::remove_dir_all(&self.dir).map_err(Error::from)
            })
            .unwrap_or_else(|e| {
                log::error!("could not remove the scratch filesystem \"{}\": {e}", self.dir)
            });
    }
}

/// The mount points below `dir`, deepest first so that nested mounts come off
/// before the ones they sit inside.
fn mounts_under(dir: &str) -> Result<Vec<String>, Error> {
    let prefix = format!("{}/", dir.trim_end_matches('/'));
    // `/proc/mounts` shows one mount per line, formatted as fields as in
    // fstab(5). The 2nd field shows the mount point. See proc_pid_mounts(5).
    // The replacements revert escaped characters. See getmntent(3) for more
    // details.
    let mut mounts: Vec<String> = std::fs::read_to_string("/proc/mounts")?
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(|point| {
            point
                .replace("\\040", " ")
                .replace("\\011", "\t")
                .replace("\\012", "\n")
                .replace("\\134", "\\")
        })
        .filter(|point| point.starts_with(&prefix))
        .collect();
    mounts.sort_by_key(|point| std::cmp::Reverse(point.len()));

    Ok(mounts)
}

/// Unmounts `mount`, lazily if it is still busy.
fn unmount(mount: &str) -> Result<(), Error> {
    let settings = SyscommandSettings { expected_code: Some(0), ..Default::default() };
    let Err(e_umount) = syscommand_timeout(["fusermount3", "-u", mount], settings.clone()) else {
        return Ok(());
    };

    log::info!("Attempting a lazy unmount, could not directly unmount {mount}: {e_umount}");

    syscommand_timeout(["fusermount3", "-uz", mount], settings)
        .map(|_| ())
        .map_err(|e| Error::runtime(format!("could not lazily unmount {mount}")).with_cause(e))
}

/// What the container used for grading a solution is created from.
#[derive(Debug)]
pub struct GradingContainerOptions {
    /// Where to create the scratch filesystem.
    pub scratch_slot: ScratchSlot,

    pub image: String,
    pub container_name: String,
    pub network_name: String,

    /// Size of the scratch filesystem, which bounds everything the container
    /// writes.
    pub disk: ByteSize,

    /// Memory the container may use, with no swap.
    pub memory: ByteSize,

    /// The directory inside the container which contains the built solution
    pub internal_build_dir: String,

    /// Where the solution is mounted.
    pub solution_dir: podman::Mount,

    /// Where the test input files are mounted.
    pub tests_dir: podman::Mount,
}

/// The container used for grading a solution, together with the network and
/// scratch storage it runs in. Dropping this struct kills every container in
/// the scratch storage.
#[derive(Debug)]
pub struct GradingContainer {
    store: ScratchStore,
    container: podman::WeakContainer,
    network: podman::WeakNetwork,

    /// The memory limit the container was created with, in bytes.
    memory: usize,

    /// The `memory.events` of the container's cgroup.
    memory_events: String,

    /// The directory inside the container which contains the built solution
    pub internal_build_dir: String,

    /// Where the solution is mounted.
    pub solution_dir: podman::Mount,

    /// Where the test input files are mounted.
    pub tests_dir: podman::Mount,
}

impl GradingContainer {
    /// Creates the scratch storage in `opts.scratch_slot`, a network inside
    /// it, and a container on that network. Fails if podman container could
    /// not be started within the specified `limit`.
    pub fn create(
        opts: &GradingContainerOptions,
        limit: Duration,
    ) -> Result<GradingContainer, Error> {
        let store = ScratchStore::create(&opts.scratch_slot, opts.disk)?;
        let network = podman::WeakNetwork::create(
            store.store(),
            opts.network_name.as_str(),
            store.network_owner(),
        )?;

        let mut container = podman::WeakContainer::new(
            opts.image.as_str(),
            opts.container_name.as_str(),
            store.store(),
        );
        container.network = Some(network.name().to_string());
        container.memory = Some(opts.memory.to_podman_string());
        container.pull_never = true;
        container.remove_on_exit = false;
        container.mounts = vec![opts.solution_dir.clone(), opts.tests_dir.clone()];
        container.start()?;
        container.block_until_started(limit)?;

        // The cgroup_path is only available when the container runs.
        let cgroup_path = podman::inspect(container.store(), container.name())?.state.cgroup_path;
        if cgroup_path.is_empty() {
            return Err(Error::runtime(format!(
                "container \"{}\" has no cgroup, memory use cannot be tracked",
                container.name()
            )));
        }
        if !cgroup_path.starts_with("/") {
            return Err(Error::runtime(format!(
                "container \"{}\" cgroup does not start with a /, got \"{}\"",
                container.name(),
                cgroup_path,
            )));
        }

        Ok(GradingContainer {
            store,
            container,
            network,
            memory: opts.memory.bytes(),
            memory_events: format!("/sys/fs/cgroup{cgroup_path}/memory.events"),
            internal_build_dir: opts.internal_build_dir.clone(),
            solution_dir: opts.solution_dir.clone(),
            tests_dir: opts.tests_dir.clone(),
        })
    }

    /// Removes what a grading container may have left under the scratch filesystem
    /// `<workspace_dir>/scratch`, unmounting directories, removing container,
    /// removing networks, and removing the scratch filesystem. If the scratch
    /// filesystem does not exist, then this will not do anything. Returns the
    /// `ScratchSlot` that was checked.
    pub fn sweep(workspace_dir: &str) -> Result<ScratchSlot, Error> {
        let workspace_dir = path_absolute(workspace_dir)?;
        let slot = ScratchSlot {
            dir: path_absolute_join(&workspace_dir, "scratch")?,
            image_store: podman::graph_root()?,
        };
        if let Some(store) = ScratchStore::open_if_exists(&slot)? {
            log::info!("removing what a previous run left in \"{}\"", slot.dir);
            drop(store);
            // Still there if the containers in it could not be killed.
            slot.check_available()?;
        }
        for mount in mounts_under(&workspace_dir)? {
            log::info!("unmounting \"{mount}\" left behind by a previous run");
            unmount(&mount)?;
        }
        Ok(slot)
    }

    /// Disconnects the container from its network.
    pub fn disconnect_network(&self) -> Result<(), Error> {
        self.network.disconnect(&self.container)
    }

    /// How many processes in the container the kernel has killed for exceeding
    /// the memory limit. This will fails once the container has exited (due to
    /// the missing cgroup).
    fn oom_kills(&self) -> Result<usize, Error> {
        // self.memory_events is a cgroup `memory.events` file containing lines
        // of `<key> <value>`: https://docs.kernel.org/admin-guide/cgroup-v2.html
        let content = std::fs::read_to_string(&self.memory_events)?;
        let count =
            content.lines().find_map(|line| line.strip_prefix("oom_kill ")).ok_or_else(|| {
                Error::runtime(format!("no oom_kill count in {}", self.memory_events))
            })?;
        count
            .trim()
            .parse()
            .map_err(|_| Error::runtime(format!("could not read oom_kill count: `{count}`")))
    }

    /// Whether the kernel has killed a process in the container for exceeding
    /// the memory limit at any point since it started, including the container
    /// itself. Blocks until podman has recorded the exit of a container that
    /// is no longer running.
    pub fn memory_exceeded(&self) -> Result<bool, Error> {
        // The kernel counts the kill before sending the signal, so a count
        // that can still be read is current.
        let read_err = match self.oom_kills() {
            Ok(count) => return Ok(count > 0),
            Err(e) => e,
        };
        // The cgroup goes with the container's init, which the kernel may
        // have picked as its victim. Podman then records the kill itself.
        let store = self.container.store();
        if podman::inspect(store, self.container.name())?.state.running {
            return Err(read_err);
        }
        self.container.wait()?;
        Ok(podman::inspect(store, self.container.name())?.state.oom_killed)
    }

    /// Checks whether the scratch filesystem has run out of space or inodes.
    pub fn disk_full(&self) -> Result<bool, Error> {
        self.store.is_full()
    }

    /// How many bytes the container can write to its scratch filesystem.
    pub fn disk_limit(&self) -> Result<usize, Error> {
        self.store.size()
    }

    /// How many bytes of RAM memory the container can use.
    pub fn memory_limit(&self) -> Result<usize, Error> {
        Ok(self.memory)
    }
}

// ----------------------------------------------------------------------------
// GradingContainer::exec functionality below, for running commands inside the
// container.
// ----------------------------------------------------------------------------

/// One command run inside the container.
#[derive(Debug, Clone, Copy)]
pub struct ExecOptions<'a> {
    /// Working directory inside the container.
    pub workdir: &'a str,

    /// The command and its arguments.
    pub cmd: &'a [&'a str],

    /// Input files, copied in and appended to the command in order.
    pub infiles: &'a [InputFile],

    /// Text piped to standard input.
    pub stdin: Option<&'a str>,

    /// Exit code to require, or `None` to return whatever was exited with.
    pub expected_code: Option<i32>,

    /// Maximum captured output per stream, in bytes.
    pub max_output: usize,

    /// Timeout in seconds.
    pub timeout: u32,
}

impl<'a> ExecOptions<'a> {
    /// The command as it runs inside the container, input files included.
    pub fn exec_cmd(&'a self) -> Vec<&'a str> {
        self.cmd
            .iter()
            .copied()
            .chain(self.infiles.iter().map(|f| f.container_path.as_str()))
            .collect()
    }
}

/// An input file of a test case, and where it ends up inside the container.
#[derive(Debug, Clone)]
pub struct InputFile {
    pub name: String,
    pub host_path: String,
    pub container_path: String,
}

impl InputFile {
    /// Assigns container names to a test case's input files. Decided once, so
    /// that whatever inspects the run afterwards refers to the same names the
    /// binary was handed.
    pub fn assign(host_paths: &[String], mount: &podman::Mount) -> Result<Vec<Self>, Error> {
        host_paths
            .iter()
            .enumerate()
            .map(|(i, host_path)| {
                let name = format!("test{i}.in");
                Ok(Self {
                    container_path: path_absolute_join(&mount.container_path, &name)?,
                    name,
                    host_path: host_path.to_owned(),
                })
            })
            .collect()
    }
}

impl GradingContainer {
    /// Runs one command in the container, copying the input files in beforehand
    /// and removing them afterwards. The command and its children are the
    /// first to be killed if the container runs out of memory.
    pub fn exec(&self, opts: &ExecOptions) -> Result<SyscommandOutput, Error> {
        // list of files to clean up on the host system after the container has
        // finished running
        let mut hostfiles_to_remove = vec![];

        for infile in opts.infiles {
            let hostfile = path_absolute_join(&self.tests_dir.host_path, &infile.name)?;
            // Copy file to the directory that is mounted into the container
            std::fs::copy(&infile.host_path, &hostfile).inspect_err(|e| {
                log::error!("Could not copy input file {} to {hostfile}: {e}", infile.host_path)
            })?;
            hostfiles_to_remove.push(hostfile);
        }

        // oom_score_adj 1000 (maximum) makes the command be targeted by the
        // kernel on OOM, rather than the container's init process.
        //
        // Note: The `sh` is just a placeholder for $0, since we only care
        // about `$@` which comes from `opts.exec_cmd()`. First argument to
        // `sh -c` just sets the command name, and the following becomes the
        // arguments.
        let mut cmd = vec!["sh", "-c", "echo 1000 > /proc/self/oom_score_adj && exec \"$@\"", "sh"];
        cmd.extend(opts.exec_cmd());
        let res = self.container.exec(
            Some(opts.workdir),
            cmd,
            SyscommandSettings {
                stdin: opts.stdin.map(String::from),
                expected_code: opts.expected_code,
                max_stdout_length: Some(opts.max_output),
                max_stderr_length: Some(opts.max_output),
                timeout: Duration::from_secs(opts.timeout.into()),
                ..Default::default()
            },
        );
        for fpath in hostfiles_to_remove {
            // Remove the file that was used in the test case
            std::fs::remove_file(&fpath)
                .unwrap_or_else(|e| log::error!("Could not remove input file \"{fpath}\": {e}"));
        }

        res
    }
}
