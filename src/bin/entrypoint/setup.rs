//! Commands an admin runs to prepare a deployment, before the autograder is
//! started for the first time. None of them are on the startup path.

use clap::Args;
use std::time::Duration;

use id2202_autograder::{
    config::{settings::KnownInstance, Settings},
    error::Error,
    podman,
};

/// The build context for the verifier image. Since it ships inside the
/// autograder image next to the sources, it is located relative to the crate
/// root.
const VERIFIER_CONTEXT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/container/verifier");

#[derive(Args, Debug)]
pub struct BuildImageArgs {
    /// Build without asking, even for a tag that is not local
    #[arg(short = 'y', long, default_value_t = false)]
    pub yes: bool,
}

/// Builds the verifier image, tagging it with `runner.podman_verifier_image`
/// value from `Settings`.
pub fn build_image(s: Settings, args: BuildImageArgs) -> Result<(), Error> {
    use std::io::Write;

    let tag = &s.runner.podman_verifier_image;

    // A tag outside `localhost/` names a registry, so it is more likely to be a
    // pre-built image that was meant to be pulled than one to build over.
    if !args.yes && !tag.starts_with("localhost/") {
        println!("Warning: \"{tag}\" is not a local tag, so it may name an image");
        println!("that is meant to be pulled rather than built here.");
        print!("Build it anyway? [y/N] ");
        std::io::stdout().flush()?;

        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("Aborted, no image was built.");
            return Ok(());
        }
    }

    println!("Building {tag} from {VERIFIER_CONTEXT}");
    podman::build(tag, VERIFIER_CONTEXT)?;
    println!("Built {tag}");
    Ok(())
}

/// Pulls the grading image named by `runner.podman_image`.
pub fn pull_image(s: Settings) -> Result<(), Error> {
    let tag = &s.runner.podman_image;

    if podman::images()?.contains(tag) {
        println!("{tag} already exists");
        return Ok(());
    }

    println!("Pulling {tag}");
    podman::pull(tag)?;
    println!("Pulled {tag}");
    Ok(())
}

#[derive(Args, Debug)]
pub struct VerifySshHostsArgs {
    /// Exit code that `ssh -T` returns on a successful GitHub connection
    #[arg(long, default_value_t = 1)]
    pub github_exit_code: i32,

    /// Exit code that `ssh -T` returns on a successful GitLab connection
    #[arg(long, default_value_t = 0)]
    pub gitlab_exit_code: i32,
}

/// Connects to every configured submission source over SSH, using the same
/// keys and known hosts file that the runner fetches with. An unknown host key
/// is presented by SSH itself, which records it once it has been accepted.
pub fn verify_ssh_hosts(s: Settings, args: VerifySshHostsArgs) -> Result<(), Error> {
    use id2202_autograder::utils::{
        create_dir_if_not_exists, path_absolute_parent, syscommand_timeout, SyscommandSettings,
    };
    use std::collections::BTreeSet;

    // SSH creates the known hosts file, but not the directory holding it.
    create_dir_if_not_exists(path_absolute_parent(&s.runner.ssh_known_hosts)?)?;

    let targets: BTreeSet<(&str, &str, u16, i32)> = s
        .submission
        .github
        .known_instances
        .iter()
        .map(|gh| (gh.ssh_user.as_str(), gh.outbound_host(), gh.ssh_port, args.github_exit_code))
        .chain(s.submission.gitlab.known_instances.iter().map(|gl| {
            (gl.ssh_user.as_str(), gl.outbound_host(), gl.ssh_port, args.gitlab_exit_code)
        }))
        .collect();

    let mut failures = 0;
    for (user, host, port, expected_code) in targets {
        let mut cmd: Vec<String> = vec![
            "ssh".to_string(),
            "-T".to_string(),
            "-p".to_string(),
            port.to_string(),
            "-o".to_string(),
            format!("UserKnownHostsFile={}", s.runner.ssh_known_hosts),
        ];
        if !s.runner.ssh_keys.is_empty() {
            cmd.extend(["-o".to_string(), "IdentitiesOnly=yes".to_string()]);
            for key in &s.runner.ssh_keys {
                cmd.extend(["-i".to_string(), key.to_owned()]);
            }
        }
        cmd.push(format!("{user}@{host}"));

        println!("Connecting to {host} on port {port}");
        let output = syscommand_timeout(
            &cmd,
            SyscommandSettings {
                // The connection is interactive when the host key is unknown.
                timeout: Duration::from_secs(300),
                ..Default::default()
            },
        )?;
        if output.code == expected_code {
            println!("{host}:{port}: ok");
        } else {
            println!(
                "{host}:{port}: failed, exited with {} rather than {expected_code}",
                output.code
            );
            failures += 1;
        }
    }

    if failures > 0 {
        return Err(Error::runtime("could not connect to every configured host"));
    }
    Ok(())
}
