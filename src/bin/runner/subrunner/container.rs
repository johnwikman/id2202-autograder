use std::time::Duration;

use id2202_autograder::{
    error::Error,
    podman::{Mount, PodmanContainer},
    utils::{path_absolute_join, SyscommandOutput, SyscommandSettings},
};

/// Information about the container used for grading a solution.
#[derive(Debug)]
pub struct ContainerInfo {
    /// The container itself. Removed when this is dropped.
    pub podman: PodmanContainer,

    /// The directory inside the container which contains the built solution
    pub internal_build_dir: String,

    /// Where the solution is mounted.
    pub solution_dir: Mount,

    /// Where the test input files are mounted.
    pub tests_dir: Mount,
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
    pub fn assign(host_paths: &[String], mount: &Mount) -> Result<Vec<Self>, Error> {
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

impl ContainerInfo {
    /// Runs one command in the container, copying the input files in beforehand
    /// and removing them afterwards.
    pub fn exec(&self, opts: &ExecOptions) -> Result<SyscommandOutput, Error> {
        // list of files to clean up on the host system after the container has
        // finished running
        let mut hostfiles_to_remove = vec![];

        for infile in opts.infiles {
            let hostfile = path_absolute_join(&self.tests_dir.host_path, &infile.name)?;
            // Copy file to the directory that is mounted into the container
            std::fs::copy(&infile.host_path, &hostfile).inspect_err(|e| {
                log::error!(
                    "Could not copy input file {} to {hostfile}: {e}",
                    infile.host_path
                )
            })?;
            hostfiles_to_remove.push(hostfile);
        }

        let res = self.podman.exec(
            Some(opts.workdir),
            opts.exec_cmd(),
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
