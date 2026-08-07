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
    mkdir -p data/containers data/log data/postgres data/ssh

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
    dotenv run python3 -m misc.test_gitlab {{SCENARIOS}}

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
    export $(cat .env)
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
