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
        tests::kind::{run_verifier::ParamValue, Kind},
        Settings, TestGroup, Tests,
    },
    error::Error,
    podman::{Mount, PodmanContainer},
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
    container: PodmanContainer,

    /// Where each verifier lives inside the container, keyed by its host path.
    paths: BTreeMap<String, String>,
}

impl Verifier {
    /// Copies every verifier referenced by `tests` into the `host_mount_path`
    /// and starts the container they run in.
    pub fn start(
        settings: &Settings,
        container_name: &str,
        host_mount_path: &str,
        tests: &Tests,
    ) -> Result<Self, Error> {
        let mount = Mount {
            host_path: host_mount_path.to_string(),
            container_path: settings.runner.mount_verifiers.clone(),
            writable: false,
        };

        std::fs::create_dir_all(&mount.host_path).map_err(|e| {
            Error::fs("creating verifier directory", &mount.host_path).with_cause(Box::new(e))
        })?;

        let mut container =
            PodmanContainer::new(&settings.runner.podman_verifier_image, container_name);

        /// Recursively collect verifiers from all defined test cases.
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

        let mut sources: BTreeSet<String> = BTreeSet::new();
        for tag in tests.tags.values() {
            for group in &tag.test_groups {
                collect_from_group(group, &mut sources);
            }
        }

        let mut paths = BTreeMap::new();
        for source in sources {
            let file = std::path::Path::new(&source);
            let (Some(stem), Some(ext)) = (file.file_stem(), file.extension()) else {
                return Err(Error::convert(format!("malformed verifier path \"{source}\"")));
            };
            // Hashed, since verifiers are not necessarily under the test config
            // root and their directory layout cannot be mirrored.
            let digest = Sha256::digest(source.as_bytes());
            let name = format!(
                "{}-{}.{}",
                stem.to_string_lossy(),
                hex::encode(&digest[..4]),
                ext.to_string_lossy(),
            );

            std::fs::copy(&source, path_absolute_join(&mount.host_path, &name)?)
                .map_err(|e| Error::fs("copying verifier", &source).with_cause(Box::new(e)))?;
            paths.insert(source, path_absolute_join(&mount.container_path, &name)?);
        }

        container.mounts = vec![mount];
        container.read_only = true;
        container.drop_privileges = true;
        container.pids_limit = Some(64);
        container.memory = Some("256m".to_string());
        container.start()?;

        Ok(Self { container, paths })
    }

    /// The path inside the container for a verifier's path on the host.
    pub fn container_path(&self, host_path: &str) -> Result<&str, Error> {
        self.paths.get(host_path).map(String::as_str).ok_or_else(|| {
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
