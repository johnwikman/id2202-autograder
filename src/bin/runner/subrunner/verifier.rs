//! Running course-provided verifier programs.
//!
//! A verifier is untrusted: it receives JSON describing one execution of a
//! student binary on stdin, and writes a verdict as JSON on stdout. Anything
//! that is not a well-formed verdict is an autograder error, never a failed
//! test case.

use base64::{prelude::BASE64_STANDARD, Engine};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use id2202_autograder::{
    config::{
        settings::PodmanImageSettings,
        tests::kind::{run_verifier::ParamValue, Kind},
        TestGroup,
    },
    error::Error,
    podman::{self, Mount, PodmanContainer},
    utils::{path_absolute_join, SyscommandSettings},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A byte string on the wire. Always tagged, so a verifier cannot silently
/// handle only the encoding its author happened to see.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "enc", content = "data", rename_all = "lowercase")]
pub enum Encoded {
    Utf8(String),
    Base64(String),
}

impl Encoded {
    pub fn new(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(s) => Self::Utf8(s.to_owned()),
            Err(_) => Self::Base64(BASE64_STANDARD.encode(bytes)),
        }
    }
}

/// What the verifier is asked to judge.
#[derive(Serialize, Debug, Clone)]
pub struct VerifierInput<'a> {
    /// The command that was run, as it appeared inside the container.
    pub cmd: &'a [&'a str],
    pub code: i32,
    pub stdout: Encoded,
    pub stderr: Encoded,
    /// Input files given to the test case, by the name they had in the
    /// container.
    pub files: BTreeMap<String, Encoded>,
    pub params: &'a BTreeMap<String, ParamValue>,
}

/// What the verifier answers.
#[derive(Deserialize, Debug, Clone)]
pub struct Verdict {
    pub accepted: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

/// The verifier programs, and the container they are run in.
#[derive(Debug)]
pub struct Verifier {
    pub container: PodmanContainer,

    /// The mounting location of where verifier scripts.
    mount: Mount,

    /// Paths to the verifiers that will be used by this verifier.
    host_paths: BTreeSet<String>,

    /// Where each verifier lives inside the container, keyed by its host path.
    internal_paths: BTreeMap<String, String>,
}

impl Verifier {
    /// Sets up the verifier image and the verifiers it should use. But never
    /// attempts to start the image itself.
    pub fn new(
        pis: &PodmanImageSettings,
        container_name: String,
        host_verifier_dir: &str,
        test_groups: &[TestGroup],
    ) -> Self {
        let mount = Mount {
            host_path: host_verifier_dir.to_string(),
            container_path: pis.mount_code.clone(),
            writable: false,
        };

        // Recursively collect paths to verifiers from all test groups.
        let mut host_paths: BTreeSet<String> = BTreeSet::new();
        fn collect_from_group(group: &TestGroup, out: &mut BTreeSet<String>) {
            for test in &group.tests {
                if let Kind::RunVerifier(conf) = &test.kind {
                    out.insert(conf.verifier_path.to_owned());
                }
            }
            for sub in &group.subgroups {
                collect_from_group(sub, out);
            }
        }
        for group in test_groups {
            collect_from_group(group, &mut host_paths);
        }

        let mut container = PodmanContainer::new(pis.image.clone(), container_name);
        container.mounts = vec![mount.clone()];
        container.read_only = true;
        container.drop_privileges = true;
        container.pids_limit = Some(64);
        container.memory = Some("256m".to_string());

        Self { container, mount, host_paths, internal_paths: BTreeMap::new() }
    }

    /// Copies all the necessary verifiers into the verifier directory and
    /// starts the verifier container.
    pub fn start(&mut self) -> Result<(), Error> {
        if !std::fs::exists(&self.mount.host_path)? {
            log::debug!("Ensuring that the verifier directory exists outside the container");
            std::fs::create_dir_all(&self.mount.host_path)?;
        }

        log::debug!(
            "Cleaning up any previous entries in the verifier dir {}",
            self.mount.host_path
        );
        for f in std::fs::read_dir(&self.mount.host_path)? {
            let path = f?.path();
            if path.is_dir() {
                std::fs::remove_dir_all(path)?;
            } else if path.is_file() {
                std::fs::remove_file(path)?;
            } else {
                return Error::err_fs(
                    "unknown file type when cleaning up verifier dir",
                    path.to_string_lossy(),
                );
            }
        }

        for p in &self.host_paths {
            let file = std::path::Path::new(p);
            let (Some(stem), Some(ext)) = (file.file_stem(), file.extension()) else {
                return Err(Error::convert(format!("malformed verifier path \"{p}\"")));
            };
            // Hashed, since verifiers are not necessarily under the test config
            // root and their directory layout cannot be mirrored.
            // (Note: paths here are absolute, so hashes should be unique.)
            let digest = Sha256::digest(p.as_bytes());
            let name = format!(
                "{}-{}.{}",
                stem.to_string_lossy(),
                hex::encode(&digest[..4]),
                ext.to_string_lossy(),
            );

            std::fs::copy(p, path_absolute_join(&self.mount.host_path, &name)?)
                .map_err(|e| Error::fs("copying verifier", p).with_cause(Box::new(e)))?;
            self.internal_paths
                .insert(p.clone(), path_absolute_join(&self.mount.container_path, &name)?);
        }

        let running_containers = podman::ps_names()?;
        if running_containers.contains(&self.container.name) {
            log::warn!("Removing dangling verifier container from a previous run");
            podman::force_rm(&self.container.name)?;
        }

        self.container.start()
    }

    /// Stops the running verifier container, but does not perform any cleanup.
    pub fn stop(&mut self) {
        self.container.stop();
    }

    /// The path inside the container for a verifier's path on the host.
    pub fn container_path(&self, host_path: &str) -> Result<&str, Error> {
        self.internal_paths.get(host_path).map(String::as_str).ok_or_else(|| {
            Error::runtime(format!(
                "verifier \"{host_path}\" was not collected before grading started"
            ))
        })
    }

    /// Runs one verifier over one blob. Every failure here is an autograder
    /// error: a verifier that cannot produce a verdict tells us nothing about
    /// the student's solution.
    pub fn verify(
        &self,
        script: &str,
        blob: &VerifierInput,
        timeout: u32,
    ) -> Result<Verdict, Error> {
        let payload = serde_json::to_string(blob)
            .map_err(|e| Error::convert("serializing verifier blob").with_cause(Box::new(e)))?;

        // Never `-O`, which strips `assert`.
        let output = self
            .container
            .exec(
                None,
                ["python3", script],
                SyscommandSettings {
                    stdin: Some(payload),
                    expected_code: Some(0),
                    max_stdout_length: Some(64 * 1024),
                    max_stderr_length: Some(64 * 1024),
                    timeout: Duration::from_secs(timeout.into()),
                    ..Default::default()
                },
            )
            .map_err(|e| {
                Error::runtime(format!("verifier \"{script}\" did not run to completion"))
                    .with_cause(Box::new(e))
            })?;

        serde_json::from_str(&output.stdout).map_err(|e| {
            Error::convert(format!("verifier \"{script}\" did not write a verdict on stdout"))
                .with_cause(Box::new(e))
        })
    }
}
