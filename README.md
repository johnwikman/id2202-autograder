# ID2202 Autograder
The autograder for the ID2202 course at KTH. This is used to run students'
solutions against a variety of test cases.

This is inteded to be run in a Linux-based container using Docker or Podman.
See further down in the README for information about how to run it as a
standalone application during development.

**NOTE: The autograder assumes a Linux-based OS and the existence of common
user space programs. Certain aspects of the autograder might break if running
on a different OS.**

## Build Docker Image

We use [`just`](https://github.com/casey/just) as the command runner. To build
the docker image:

```sh
# (sudo can be omitted if running rootless docker)
sudo just build-image
```

## Test Structure
The structure of the tests cases are shown in `example/tests`. At the root of this directory sits the file `tests.toml` that specify overarching test configuration such as default values and grading tags.

The configuration is specified in a hierarchical TOML structure. A test case is specified as a file ending with `.test.toml`. That file can specify every detail of the test case. Anything that is not specified is inherited by a `config.toml` file located in the same directory. This inheritence is repeated until the root of the test directory.

A test is specified as follows:

```toml
[test]
kind = "run"

# Options of how the test is supposed to be run. These options are specific to
# to specified kind above.
[test.options]
bin = "my_echo"
code = [0]
stdin = "Hello"
stdout = ["Hello"]
```


## Runtime Structure
The autograder is structured into 3 binaries: `entrypoint`, `runner`, and `server`.

 * The `entrypoint` binary starts up a process whole sole process is to perform simple tests and manage the runner and server processes. This should be the only process that is manually invoked from the command line.
 * The `runner` binary starts up a runner process that can run incoming jobs. Each runner process will only run a single job at a time.
 * The `server` binary starts up a web server that is responsible for serving web pages and accepting incoming REST API calls. These calls are used to trigger a job on the autograder.

An incoming grading submission, after being validated, is inserted into a postgres database by the server process. The submission is then picked up by a runner process and graded. This setup is used to prevent silent errors, where the submitter is notified if the runner process crashed before it finished grading.

A diagram to illustrate the setup (entrypoint omitted):

```
   ┌──────────────────────────────────────────────────────────┐
   │                          GitHub                          │
   └──────────────────────────────────────────────────────────┘
      │            Ʌ                                    Ʌ
[1. Push Hook]     │                                    │
      │     [3. Submit ACK]              [6. Send Submission Results]
      V            │                                    │
    ┌────────────────┐                     ┌─────────┐  │
    │ Server Process │───[4. Notify]───┬──>│ Runner  │──┤
    └────────────────┘              ┌──│───│ Proc. 0 │  │
                │                   │  │   └─────────┘  │
     [2. Register Submission]       │  │   ┌─────────┐  │
                │                   │  ├──>│ Runner  │──┤
                │   ┌────────────┐  ├──│───│ Proc. 1 │  │
                └──>│ PostgreSQL │<─┤  │   └─────────┘  │
                    │  Database  │  │  │   ┌─────────┐  │
                    └────────────┘  │  └──>│ Runner  │──┘
                                    └──────│ Proc. 2 │
                          [5. Fetch Job]   └─────────┘
                                               ...
```

## Development Practice

Development is easiest when running the autograder locally on your computer.

### Dev Dependencies

The following dependencies are required to install on your system:

* Rust (`cargo` should be present on the path)
* PostgreSQL client libraries (e.g. `pacman -S postgresql-libs` on Arch Linux)
* Docker
* Docker Compose

Run the following in the terminal to install rust dependencies:

```sh
cargo build
cargo install diesel_cli --no-default-features --features "postgres"
```

### Dev Workflow

Development is best done by running the rust code on the host environment. The
following steps will set up the development environment:

```sh
# Setup docker compose directories
just setup-dirs

# Setup .env file (needed by diesel cli)
#
# For safety's sake, the GitHub auth token is placed here as well to avoid it
# being committed to the repository by mistake. Replace <github_token> by your
# personal access token for the GitHub instance you are testing on.
cat <<EOF > .env
DATABASE_URL=postgres://autograder:ChangeMe@localhost/autograder
AUTOGRADER_GITHUB_AUTH_TOKENS=<domain>=<github_token>
EOF

# Start the database
sudo docker compose up -d --remove-orphans postgres

# Setup autograder tables using diesel
# (Only necessary to do the first time or if a new migration has been added)
diesel migration run
```

To start a local instance of the autograder:

```sh
# Ensures that the GitHub token is available to the autograder
export $(cat .env)

# Start the autograder
./target/debug/entrypoint -s example/settings.toml start
```

### Notes on Setting Up The GitLab Instance

**IMPORTANT: These notes are for development/testing only, and are not
suitable for a production environment.** 

A small GitLab Community Edition instance is preconfigured in the
`compose.yaml` file. The instance will be available over port 8929 for HTTP and
over port 2424 for SSH. However, some configuration needs to be done manually
on first startup:

 * The external hostname for the autograder needs to be added to the allowed
   network hosts. See the following for more information:
   https://docs.gitlab.com/security/webhooks/#allow-outbound-requests-to-certain-ip-addresses-and-domains
    1. Log in as the `root` user.
    2. Click `Admin` in top right.
    3. On the left, select `Settings` > `Network`.
    4. Under `Outbound requests`, add `host.docker.internal:8080`.

 * The webhook can now be added to your GitLab repositories. Add the webhook
   URL `http://host.docker.internal:8080/api/submit/gitlab`. Make sure that the
   following options are properly set:

    - Set secret token to `s3cr3t`.
    - Trigger on push events.
    - Disable SSL verification.


When starting the autograder to test with a local GitLab instance, make sure to
configure that the server listens on IP address `0.0.0.0` (otherwise
inaccessible from the docker container):

```sh
export $(cat .env)
AUTOGRADER_SERVER_ADDRESS=0.0.0.0 ./target/debug/entrypoint -s example/settings.toml start
```
