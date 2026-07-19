"""Support code for the GitLab test suite."""

import json
import os
import subprocess
import tempfile
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

# "canceled" is what the autograder reports for AutograderFailure, i.e. a bug
# in the autograder rather than a bad submission.
TERMINAL_STATUSES = ("success", "failed", "canceled", "skipped")

# The suite pushes as an ordinary group member rather than as root, so that it
# is subject to the same permissions a student would be. The key is never
# generated here; `ensure_test_user` says how to create it.
TEST_USER = "itest"
SSH_KEY = REPO_ROOT / "data" / "ssh" / "itest_ed25519"
KNOWN_HOSTS = REPO_ROOT / "data" / "ssh" / "itest_known_hosts"


@dataclass
class Config:
    """Read from the settings file the autograder itself was started with, so
    the two cannot drift apart."""

    domain: str  # host[:port], as matched against known_instances
    gitlab_api: str
    secret: str
    namespace: str
    prefix: str
    autograder_api: str
    webhook_url: str
    gitlab_token: str
    autograder_token: str
    api_token: str

    @classmethod
    def load(cls, settings_path):
        settings = tomllib.loads(Path(settings_path).read_text())
        instance = settings["submission"]["gitlab"]["known_instances"][0]
        domain = instance["domain"]
        scheme = "https" if instance.get("use_https") else "http"

        # An admin token, since the suite creates the user it pushes as. The
        # autograder's own token is a different, non-admin identity, and the
        # suite needs it too, to grant that user access to the test group.
        token = os.environ.get("GITLAB_ROOT_TOKEN")
        if not token:
            raise SystemExit("GITLAB_ROOT_TOKEN is not set. Run: just setup-gitlab-tokens")

        pairs = (p.partition("=") for p in os.environ.get("AUTOGRADER_GITLAB_AUTH_TOKENS", "").split(";"))
        autograder_token = next((t.strip() for d, _, t in pairs if d.strip() == domain), "")
        if not autograder_token:
            raise SystemExit(f"AUTOGRADER_GITLAB_AUTH_TOKENS has no entry for {domain}")
        if autograder_token == token:
            raise SystemExit(
                "the autograder is configured with the root token. It should have its "
                "own non-admin one; see GITLAB_AUTOGRADER_TOKEN in the justfile."
            )

        api_tokens = settings["server"]["secrets"]["api_auth_tokens"]
        api_token = os.environ.get("AUTOGRADER_SERVER_API_AUTH_TOKENS", "").split(";")[0] or (
            api_tokens[0] if api_tokens else ""
        )
        if not api_token:
            raise SystemExit(
                "no autograder API token. Export AUTOGRADER_SERVER_API_AUTH_TOKENS for "
                "both the autograder and this shell."
            )

        port = int(os.environ.get("AUTOGRADER_SERVER_PORT", settings["server"]["port"]))
        address = os.environ.get("AUTOGRADER_SERVER_ADDRESS", settings["server"]["address"])
        return cls(
            domain=domain,
            gitlab_api=f"{scheme}://{domain}/api/v4",
            secret=settings["submission"]["gitlab"]["webhook_secret"],
            namespace=instance["allowed_namespaces"][0],
            prefix=(instance["allowed_repo_prefixes"] or [""])[0],
            # The suite reaches the autograder over the loopback; GitLab reaches it
            # from inside its container, which is what the webhook URL has to name.
            autograder_api=f"http://{'127.0.0.1' if address == '0.0.0.0' else address}:{port}/api",
            webhook_url=f"http://host.docker.internal:{port}/api/submit/gitlab",
            gitlab_token=token,
            autograder_token=autograder_token,
            api_token=api_token,
        )


def http(method, url, *, headers=None, body=None):
    """Returns (status, parsed body)."""
    headers = dict(headers or {})
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        headers.setdefault("Content-Type", "application/json")
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            raw, status = resp.read(), resp.status
    except urllib.error.HTTPError as e:
        raw, status = e.read(), e.code
    except urllib.error.URLError as e:
        raise SystemExit(f"could not reach {url}: {e.reason}")
    try:
        return status, json.loads(raw)
    except ValueError:
        return status, raw.decode(errors="replace")


def git(*args, cwd):
    """A git command that is expected to work."""
    ssh = (
        # `IdentitiesOnly` keeps ssh from offering whatever the agent holds, so
        # the push is always attributed to the test user. The known_hosts file
        # is the suite's own, so a run never touches ~/.ssh. `BatchMode` turns
        # a missing prerequisite into an error rather than a prompt.
        f"core.sshCommand=ssh -o BatchMode=yes -i {SSH_KEY} -o IdentitiesOnly=yes"
        f" -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile={KNOWN_HOSTS}"
    )
    result = subprocess.run(
        ["git", "-c", ssh, *args],
        cwd=cwd,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise SystemExit(f"git {' '.join(args)} failed:\n{result.stderr.strip()}")
    return result.stdout.strip()


def gitlab(cfg: Config, method, path, body=None):
    """A GitLab API call that is expected to work."""
    status, parsed = http(
        method,
        f"{cfg.gitlab_api}{path}",
        headers={"PRIVATE-TOKEN": cfg.gitlab_token},
        body=body,
    )
    if not 200 <= status < 300:
        raise SystemExit(f"GitLab {method} {path}: {status} {parsed}")
    return parsed


def ensure_setup(cfg: Config):
    """Brings GitLab to the state the scenarios assume. Create-once and safe to
    repeat; the only thing it will not do for you is generate the SSH key."""
    SSH_KEY.parent.mkdir(parents=True, exist_ok=True)
    pubkey_path = SSH_KEY.with_suffix(".pub")
    if not (SSH_KEY.is_file() and pubkey_path.is_file()):
        raise SystemExit(
            "missing test SSH key. Create one with:\n"
            f'  ssh-keygen -t ed25519 -N "" -f {SSH_KEY.relative_to(REPO_ROOT)}'
        )

    # Without this GitLab refuses to deliver webhooks to the host.
    gitlab(
        cfg,
        "PUT",
        "/application/settings",
        {"allow_local_requests_from_web_hooks_and_services": True},
    )

    groups = gitlab(cfg, "GET", f"/groups?search={urllib.parse.quote(cfg.namespace)}")
    group = next((g for g in groups if g["full_path"] == cfg.namespace), None)
    if group is None:
        group = gitlab(cfg, "POST", "/groups", {"name": cfg.namespace, "path": cfg.namespace})
        print(f"created group {cfg.namespace}")

    found = gitlab(cfg, "GET", f"/users?username={TEST_USER}")
    if found:
        user = found[0]
    else:
        user = gitlab(
            cfg,
            "POST",
            "/users",
            {
                "username": TEST_USER,
                "name": "Autograder Test Suite",
                "email": f"{TEST_USER}@example.com",
                "force_random_password": True,
                "skip_confirmation": True,
            },
        )
        print(f"created user {TEST_USER}")

    # The autograder fetches the repository and posts commit statuses as its own
    # user, so it needs access to the group the test projects live in.
    status, autograder = http(
        "GET", f"{cfg.gitlab_api}/user", headers={"PRIVATE-TOKEN": cfg.autograder_token}
    )
    if status != 200:
        raise SystemExit(f"the autograder's GitLab token is not usable: {status} {autograder}")

    members = {m["id"] for m in gitlab(cfg, "GET", f"/groups/{group['id']}/members?per_page=100")}
    # Maintainer for the pusher, since default branch protection lets only
    # maintainers push to `main` on a fresh project. Developer is enough for the
    # autograder to fetch and to set commit statuses.
    for member, level in ((user, 40), (autograder, 30)):
        if member["id"] not in members:
            gitlab(
                cfg,
                "POST",
                f"/groups/{group['id']}/members",
                {"user_id": member["id"], "access_level": level},
            )
            print(f"added {member['username']} to {cfg.namespace}")

    # Compare the key material, not the whole line, which carries a comment.
    pubkey = pubkey_path.read_text().strip()
    registered = gitlab(cfg, "GET", f"/users/{user['id']}/keys")
    if not any(k["key"].split()[1] == pubkey.split()[1] for k in registered):
        gitlab(
            cfg,
            "POST",
            f"/users/{user['id']}/keys",
            {"title": "autograder test suite", "key": pubkey},
        )
        print(f"registered {pubkey_path.name} with {TEST_USER}")


@dataclass
class Context:
    cfg: Config
    projects: list = field(default_factory=list)
    # Last submission this scenario produced, so a failure can show its report.
    submission_id: int = None

    def api(self, path):
        """An authenticated call to the autograder's own API."""
        status, parsed = http(
            "GET",
            f"{self.cfg.autograder_api}{path}",
            headers={"Authorization": f"Bearer {self.cfg.api_token}"},
        )
        assert status == 200, f"GET {path}: {status} {parsed}"
        return parsed

    def create_project(self, slug):
        """An empty project in the configured namespace, with a push webhook
        pointing at the autograder. Unique per run, so leftovers from an earlier
        run cannot be mistaken for this one's."""
        # `name` and `path` are identical on purpose: the submit handler drops
        # the push when the name from `path_with_namespace` differs from
        # `project.name`, and the calls back to GitLab address the project as
        # `<namespace>/<name>`.
        name = f"{self.cfg.prefix}itest-{int(time.time())}-{slug}"
        group = gitlab(
            self.cfg, "GET", f"/groups/{urllib.parse.quote(self.cfg.namespace, safe='')}"
        )
        project = gitlab(
            self.cfg,
            "POST",
            "/projects",
            {
                "name": name,
                "path": name,
                "namespace_id": group["id"],
                "visibility": "private",
                "initialize_with_readme": False,
            },
        )
        self.projects.append(project)
        print(f"    created {project['path_with_namespace']}")

        gitlab(
            self.cfg,
            "POST",
            f"/projects/{project['id']}/hooks",
            {
                "url": self.cfg.webhook_url,
                "token": self.cfg.secret,
                "push_events": True,
                "enable_ssl_verification": False,
            },
        )
        return project

    def push(self, project, files, message, branch="main"):
        """Pushes `files` (path -> bytes/str) as one commit over SSH, the way a
        student submits."""
        with tempfile.TemporaryDirectory() as tmp:
            for path, content in files.items():
                dest = Path(tmp) / path
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_bytes(content if isinstance(content, bytes) else content.encode())

            git("init", "-q", "-b", branch, cwd=tmp)
            git("add", "-A", cwd=tmp)
            git(
                "-c", "user.name=itest",
                "-c", "user.email=itest@example.com",
                "commit", "-q", "-m", message,
                cwd=tmp,
            )
            git("remote", "add", "origin", project["ssh_url_to_repo"], cwd=tmp)
            git("push", "-q", "origin", branch, cwd=tmp)
            sha = git("rev-parse", "HEAD", cwd=tmp)

        print(f"    pushed {sha[:12]}: {message!r}")
        return sha

    def wait_for_submission(self, sha, timeout=120):
        """The submission the autograder registered for this commit, which is
        the direct evidence that the webhook arrived."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            found = self.api(f"/submission?source_kind=gitlab&commit_hash={sha}")["items"]
            if found:
                self.submission_id = found[0]["submission_id"]
                print(f"    submission {self.submission_id}")
                return self.submission_id
            time.sleep(3)
        raise AssertionError(
            f"no submission was registered for {sha[:12]} within {timeout}s: the "
            "webhook never arrived"
        )

    def wait_for_status(self, project, sha, timeout=600):
        """Blocks until GitLab carries a terminal commit status, which only
        happens once the autograder has reported back."""
        deadline = time.monotonic() + timeout
        last = None
        while time.monotonic() < deadline:
            statuses = gitlab(
                self.cfg, "GET", f"/projects/{project['id']}/repository/commits/{sha}/statuses"
            )
            status = max(statuses, key=lambda s: s["id"]) if statuses else None
            if status and status["status"] != last:
                last = status["status"]
                print(f"    status: {last}")
            if last in TERMINAL_STATUSES:
                return last
            time.sleep(5)
        raise AssertionError(f"commit {sha[:12]} stuck at {last or 'no status'} after {timeout}s")

    def files_from(self, source, prefix):
        """A directory tree, relative to the repository root, as `push` wants
        it."""
        root = REPO_ROOT / source
        return {
            f"{prefix}/{p.relative_to(root)}": p.read_bytes()
            for p in sorted(root.rglob("*"))
            if p.is_file()
        }

    def cleanup(self):
        for project in self.projects:
            gitlab(self.cfg, "DELETE", f"/projects/{project['id']}")
