# Read environment variables from `.env`.
set dotenv-load := true

image_version := "2.0.0-dev4"
image_name    := "localhost/id2202-autograder"
image_tag     := image_name + ":" + image_version

build-image:
    docker build \
        -t {{image_tag}} \
        --build-arg "CARGO_BUILD_FLAGS=--release" \
        .

rm-image:
    docker rmi {{image_tag}}

setup-dirs:
    mkdir -p data/containers data/log data/shadow data/ssh \
        data/postgres \
        data/gitlab/config data/gitlab/logs data/gitlab/data

# Generate the static HTML documentation.
gen-docs settings="example/settings.toml" output_dir="target/docs/site":
    cargo build --features docs --bin server --bin docgen
    rm -rf "{{output_dir}}"
    mkdir -p "{{output_dir}}"
    ./target/debug/server -s "{{settings}}" emit-openapi "{{output_dir}}/openapi.json"
    ./target/debug/docgen -s "{{settings}}" --out "{{output_dir}}" --openapi "{{output_dir}}/openapi.json"
    @echo "Documentation generated. Open {{output_dir}}/index.html"

# Run the basic test suite: Unit tests + make sure that clippy does not complain.
test-basic:
    cargo test
    cargo clippy

# Run the GitLab test suite against an already running stack. Optionally takes
# the names of the scenarios to run. See misc/test_gitlab/__main__.py.
test-gitlab *SCENARIOS:
    python3 -m misc.test_gitlab {{SCENARIOS}}

setup-sshkeys:
    #!/usr/bin/env bash
    KEYS=( id_ed25519 itest_ed25519 )
    for key in ${KEYS[@]}; do
        if [ ! -f data/ssh/$key ]; then
            ssh-keygen -t ed25519 -N "" -f data/ssh/$key
            echo -e "created: data/ssh/$key"
        else
            echo -e "already exists: data/ssh/$key"
        fi
    done

# Sets up the GitLab instance with an autograder user, together with API tokens
# for the autograder user and the root user. The following environment
# variables should be set inside the .env file:
#  - `GITLAB_ROOT_TOKEN`: API token for the root user
#  - `GITLAB_AUTOGRADER_TOKEN`: API token for autograder user
#  - `GITLAB_AUTOGRADER_PASSWORD`: password for autograder user
setup-gitlab:
    #!/usr/bin/env bash
    set -euo pipefail
    KEY_FILE="data/ssh/id_ed25519.pub"
    if [[ ! -f "${KEY_FILE}" ]]; then
      echo "missing ${KEY_FILE}." >&2
      exit 1
    fi
    sudo docker compose exec -T \
      -e ROOT_TOKEN="${GITLAB_ROOT_TOKEN}" \
      -e AUTOGRADER_TOKEN="${GITLAB_AUTOGRADER_TOKEN}" \
      -e AUTOGRADER_PASSWORD="${GITLAB_AUTOGRADER_PASSWORD}" \
      gitlab gitlab-rails runner '
        TOKEN_NAME = "id2202"
        VALIDITY = 300.days

        def set_password(u, password)
          u.password = password
          u.password_confirmation = password
          u.password_automatically_set = false
          u.password_expires_at = nil
          u.save!
        end

        def sync_token(u, scopes, value)
          existing = PersonalAccessToken.find_by_token(value)
          if existing && existing.user_id == u.id
            existing.update!(name: TOKEN_NAME, scopes: scopes, revoked: false,
                             expires_at: VALIDITY.from_now)
            puts "#{u.username}: token extended to #{existing.expires_at}"
          else
            u.personal_access_tokens.where(name: TOKEN_NAME).delete_all
            t = u.personal_access_tokens.create!(scopes: scopes, name: TOKEN_NAME,
                                                 expires_at: VALIDITY.from_now)
            t.set_token(value)
            t.save!
            puts "#{u.username}: token created"
          end
        end

        # Without this GitLab refuses to deliver webhooks to the autograder.
        # (See README.md for more details.)
        ApplicationSetting.current.update!(outbound_local_requests_whitelist: ["host.docker.internal:8080"])
        puts "instance: webhooks to the autograder allowed"

        root = User.find_by_username("root") or abort("no such GitLab user: root")

        autograder = User.find_by_username("autograder")
        if autograder.nil?
          Users::CreateService.new(nil,
            username: "autograder", name: "Autograder",
            email: "autograder@localhost",
            password: ENV["AUTOGRADER_PASSWORD"],
            password_confirmation: ENV["AUTOGRADER_PASSWORD"],
            organization_id: Organizations::Organization.first.id,
            skip_confirmation: true
          ).execute
          autograder = User.find_by_username("autograder") or abort("could not create GitLab user: autograder")
          puts "autograder: user created and password set"
        else
          set_password(autograder, ENV["AUTOGRADER_PASSWORD"])
          puts "autograder: password set"
        end

        sync_token(root, ["api", "admin_mode"], ENV["ROOT_TOKEN"])
        sync_token(autograder, ["api"], ENV["AUTOGRADER_TOKEN"])
      '
    GITLAB_API="http://localhost:8929/api/v4"
    if curl -sf -H "PRIVATE-TOKEN: ${GITLAB_AUTOGRADER_TOKEN}" "${GITLAB_API}/user/keys" \
         | grep -qF "$(cut -d ' ' -f2 "${KEY_FILE}")"; then
      echo "autograder: SSH key already registered"
    else
      curl -sf -H "PRIVATE-TOKEN: ${GITLAB_AUTOGRADER_TOKEN}" \
        --data-urlencode "title=id2202 itest" \
        --data-urlencode "key=$(cat "${KEY_FILE}")" \
        "${GITLAB_API}/user/keys" && echo
      echo "autograder: SSH key registered"
    fi


#  .+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.
# (                                                         )
#  )     888b     d888                   d8b               (
# (      8888b   d8888                   Y8P                )
#  )     88888b.d88888                                     (
# (      888Y88888P888  8888b.   .d88b.  888  .d8888b       )
#  )     888 Y888P 888     "88b d88P"88b 888 d88P"         (
# (      888  Y8P  888 .d888888 888  888 888 888            )
#  )     888   "   888 888  888 Y88b 888 888 Y88b.         (
# (      888       888 "Y888888  "Y88888 888  "Y8888P       )
#  )                                 888                   (
# (                             Y8b d88P                    )
#  "+.+"+.+"+.+"+.+"+.+"+.+"+.+"+"Y88P""+.+"+.+"+.+"+.+"+.+"
#
# A "magic" setup of the autograder, where passwords and config is chosen for
# the user. This should never be used in a production environment. Only for
# familiarizing oneself with the autograder test configuration and its API.
# This is a non-interactive process by default, but can prompt the user if
# things arise.
magic-setup:
    # Check whether we need sudo to interact with the docker engine.
    @echo "checking how to reach the docker engine (this may ask for your sudo password)"
    @if docker info >/dev/null 2>&1; then \
         dotenv set MAGIC_SUDO "" && echo "reachable as $(whoami): docker engine"; \
     elif sudo docker info >/dev/null 2>&1; then \
         dotenv set MAGIC_SUDO "sudo" && echo "reachable with sudo: docker engine"; \
     else \
         echo "cannot reach the docker engine. Is docker installed and running?" >&2; \
         exit 1; \
     fi
    # An earlier magic instance is otherwise reused as-is, which is rarely what
    # we want if it was left half-finished.
    @if [ ! -e data/gitlab ]; then \
         echo "not present: data/gitlab, skipping removal check"; \
     else \
         read -rp "data/ holds a previous magic instance, remove it? [y/N] " ans; \
         case "$ans" in \
         [yY]*) $(dotenv get MAGIC_SUDO) rm -rf data && echo "removed: data/";; \
         *) echo "kept: data/";; \
         esac; \
     fi
    just setup-dirs
    just setup-sshkeys

    # Set up dummy credentials in .env. DO NOT USE THESE IN PRODUCTION.
    dotenv set GITLAB_ROOT_TOKEN "glpat-FHGY8yS6oxcn9KM9WmdDE7"
    dotenv set GITLAB_AUTOGRADER_TOKEN "glpat-Hmmn72ezZ53PHB6zLt6pPf"
    dotenv set GITLAB_AUTOGRADER_PASSWORD "Kwccyk7iBQ8"
    dotenv set AUTOGRADER_SERVER_API_AUTH_TOKENS "example-api-token"
    dotenv set AUTOGRADER_GITLAB_AUTH_TOKENS \
         "localhost:8929=$(dotenv get GITLAB_AUTOGRADER_TOKEN)"

    # Build the autograder image
    $(dotenv get MAGIC_SUDO) just build-image
    $(dotenv get MAGIC_SUDO) docker compose up -d --remove-orphans postgres gitlab
    # Postgres only starts listening on TCP once it is done initializing the
    # database, which takes a moment on a fresh data/postgres.
    @echo -n "waiting for postgres to accept connections"
    @until $(dotenv get MAGIC_SUDO) docker compose exec -T postgres \
         pg_isready -h 127.0.0.1 -U autograder >/dev/null 2>&1; do \
         sleep 1 && echo -n "."; \
     done; echo -e "\npostgres is up"
    # Setup database tables
    $(dotenv get MAGIC_SUDO) docker compose run --rm \
        -e "DATABASE_URL=postgres://autograder:ChangeMe@postgres/autograder" \
        autograder diesel migration run
    # Fetches and builds necessary images
    $(dotenv get MAGIC_SUDO) docker compose run --rm --no-deps autograder \
        /autograder/target/release/entrypoint \
        --settings /mnt/example/settings.toml pull-image
    $(dotenv get MAGIC_SUDO) docker compose run --rm --no-deps autograder \
        /autograder/target/release/entrypoint \
        --settings /mnt/example/settings.toml build-image

    # A freshly created GitLab needs a few minutes before it answers, so
    # hopefully it should be up by now.
    @echo -n "waiting for gitlab to come up"
    @until curl -sf -o /dev/null http://localhost:8929/users/sign_in; do \
         sleep 5 && echo -n "."; \
     done; echo -e "\ngitlab is up"
    just setup-gitlab

    # Now verify SSH hosts when everything is properly set up
    $(dotenv get MAGIC_SUDO) docker compose run --rm --no-deps autograder \
        /autograder/target/release/entrypoint \
        --settings /mnt/example/settings.toml verify-ssh-hosts

    $(dotenv get MAGIC_SUDO) docker compose down
    @echo "setup done. Start the instance with 'just magic-start'"

# Start the magic instance in the background. Requires a prior `magic-setup`.
magic-start:
    @test -n "${MAGIC_SUDO+isset}" || { \
         echo "no magic instance here yet, run 'just magic-setup' first" >&2; \
         exit 1; \
     }
    $MAGIC_SUDO docker compose up -d --remove-orphans postgres gitlab autograder
    @echo -n "waiting for gitlab to come up"
    @until curl -sf -o /dev/null http://localhost:8929/users/sign_in; do \
         sleep 5 && echo -n "."; \
     done; echo -e "\ngitlab is up"
    @echo "autograder: http://localhost:8080"
    @echo "gitlab:     http://localhost:8929"

# Stop the magic instance. Everything under `data/` is kept for the next start.
magic-stop:
    @test -n "${MAGIC_SUDO+isset}" || { \
         echo "no magic instance here yet, run 'just magic-setup' first" >&2; \
         exit 1; \
     }
    $MAGIC_SUDO docker compose down
