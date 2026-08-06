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

        root = User.find_by_username("root") or abort("no such GitLab user: root")

        autograder = User.find_by_username("autograder")
        if autograder.nil?
          autograder = User.new(username: "autograder", name: "Autograder",
                                email: "autograder@localhost")
          autograder.skip_confirmation!
          puts "autograder: user created"
        end
        set_password(autograder, ENV["AUTOGRADER_PASSWORD"])
        puts "autograder: password set"

        sync_token(root, ["api", "admin_mode"], ENV["ROOT_TOKEN"])
        sync_token(autograder, ["api"], ENV["AUTOGRADER_TOKEN"])
      '
