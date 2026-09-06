//! Commands an admin runs to prepare a deployment, before the autograder is
//! started for the first time. None of them are on the startup path.

use clap::Args;
use std::time::Duration;

use id2202_autograder::{
    config::{settings::KnownInstance, Settings},
    error::Error,
    podman,
};

#[derive(Args, Debug)]
pub struct SetupImageArgs {
    /// Automatically accept any prompt that would usually ask for permission.
    #[arg(short = 'y', long, default_value_t = false)]
    pub yes: bool,
}

/// Sets up the images that can be used inside the autograder. If they have a
/// build process provided, then it may attempt to build that image.
pub fn setup_images(s: Settings, args: SetupImageArgs) -> Result<(), Error> {
    use std::io::Write;

    for (name, img) in &s.runner.podman.images {
        if podman::images()?.contains(&img.image) {
            println!("{name}: {} already exists", img.image);
            continue;
        }

        if let Some(build) = &img.build {
            let mut do_build = args.yes;
            if !do_build {
                println!("{name}: Build config exists for {}.", img.image);
                print!("{name}: Build the image instead of attempting a pull? [Y/n] ");
                std::io::stdout().flush()?;

                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if matches!(answer.trim().to_lowercase().as_str(), "" | "y" | "yes") {
                    do_build = true;
                }
            }

            if do_build {
                println!("{name}: Building {}.", img.image);
                podman::build(&img.image, &build.path, Some(&build.file))?;
                continue;
            }
        }

        println!("{name}: Pulling {}.", img.image);
        podman::pull(&img.image)?;
    }

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
