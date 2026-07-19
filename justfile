image_version := "2.0.0-dev3"
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
    python3 -m misc.test_gitlab {{SCENARIOS}}

# Sync GitLab's access tokens with the values in .env, so that they survive a
# recreation of the GitLab instance. The test suite needs admin_mode to create
# its own user; the autograder deliberately gets a plain non-admin token.
setup-gitlab-tokens:
    #!/usr/bin/env bash
    set -euo pipefail
    source .env
    sudo docker compose exec -T \
      -e ROOT_TOKEN="${GITLAB_ROOT_TOKEN}" \
      -e AUTOGRADER_TOKEN="${GITLAB_AUTOGRADER_TOKEN}" \
      gitlab gitlab-rails runner '
        def sync(username, scopes, value)
          u = User.find_by_username(username) or abort("no such GitLab user: #{username}")
          u.personal_access_tokens.where(name: "id2202").delete_all
          t = u.personal_access_tokens.create!(scopes: scopes, name: "id2202",
                                               expires_at: 300.days.from_now)
          t.set_token(value)
          t.save!
          puts "#{username}: ok"
        end
        sync("root", ["api", "admin_mode"], ENV["ROOT_TOKEN"])
        sync("autograder", ["api"], ENV["AUTOGRADER_TOKEN"])
      '
