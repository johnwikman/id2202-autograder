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
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

# "canceled" is what the autograder reports for AutograderFailure, i.e. a bug
# in the autograder rather than a bad submission.
TERMINAL_STATUSES = ("success", "failed", "canceled", "skipped")

# The suite pushes as an ordinary group member rather than as root, so that it
# is subject to the same permissions a student would be. The key is never
# generated here; `Config.load` says how to create it.
TEST_USER = "itest"
SSH_KEY = REPO_ROOT / "data" / "ssh" / "itest_ed25519"
KNOWN_HOSTS = REPO_ROOT / "data" / "ssh" / "itest_known_hosts"


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


@dataclass
class Config:
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
        """`settings_path` should point to the same settings file the autograder was started with."""
        SSH_KEY.parent.mkdir(parents=True, exist_ok=True)
        if not (SSH_KEY.is_file() and SSH_KEY.with_suffix(".pub").is_file()):
            raise SystemExit(
                "missing test SSH key. Create one with:\n"
                f'  ssh-keygen -t ed25519 -N "" -f {SSH_KEY.relative_to(REPO_ROOT)}'
            )

        settings = tomllib.loads(Path(settings_path).read_text())
        instance = settings["submission"]["gitlab"]["known_instances"][0]
        domain = instance["domain"]
        scheme = "https" if instance.get("use_https") else "http"

        token = os.environ.get("GITLAB_ROOT_TOKEN")
        if not token:
            raise SystemExit("GITLAB_ROOT_TOKEN is not set.")

        pairs = (p.partition("=") for p in os.environ.get("AUTOGRADER_GITLAB_AUTH_TOKENS", "").split(";"))
        autograder_token = next((t.strip() for d, _, t in pairs if d.strip() == domain), "")
        if not autograder_token:
            raise SystemExit(f"AUTOGRADER_GITLAB_AUTH_TOKENS has no entry for {domain}")
        if autograder_token == token:
            raise SystemExit(
                "Autograder is configured with the root token. It should have its "
                "own non-root token, specified by GITLAB_AUTOGRADER_TOKEN."
            )

        api_tokens = settings["server"]["secrets"]["api_auth_tokens"]
        api_token = os.environ.get("AUTOGRADER_SERVER_API_AUTH_TOKENS", "").split(";")[0] or (
            api_tokens[0] if api_tokens else ""
        )
        if not api_token:
            raise SystemExit(
                "No autograder API token. Export AUTOGRADER_SERVER_API_AUTH_TOKENS for "
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


    def request(self, target, path, method="GET", body=None, token=None):
        """Returns (status, parsed). `target` is "gitlab" or "autograder"."""
        if target == "gitlab":
            url = f"{self.gitlab_api}{path}"
            headers = {"PRIVATE-TOKEN": token or self.gitlab_token}
        else:
            url = f"{self.autograder_api}{path}"
            headers = {"Authorization": f"Bearer {token or self.api_token}"}
        return http(method, url, headers=headers, body=body)

    def gitlab(self, method, path, body=None, token=None):
        """A GitLab API call that is expected to work."""
        status, parsed = self.request("gitlab", path, method, body, token)
        if not 200 <= status < 300:
            raise SystemExit(f"GitLab {method} {path}: {status} {parsed}")
        return parsed

    def api(self, path):
        """An authenticated call to the autograder's own API."""
        status, parsed = self.request("autograder", path)
        assert status == 200, f"GET {path}: {status} {parsed}"
        return parsed

    def request_until(self, target, path, *, until, method="GET", body=None,
                      token=None, timeout=60, interval=0.5):
        """A call that is expected to work once `until(status, parsed)` holds."""
        deadline = time.monotonic() + timeout
        status, parsed = None, None
        while time.monotonic() < deadline:
            status, parsed = self.request(target, path, method, body, token)
            if until(status, parsed):
                if not 200 <= status < 300:
                    raise SystemExit(f"{target} {method} {path}: {status} {parsed}")
                return parsed
            time.sleep(interval)
        raise SystemExit(f"{target} {method} {path} still {status} after {timeout}s: {parsed}")


class Context:
    def __init__(self, cfg: Config):
        """Brings GitLab to the state the scenarios assume. Create-once and safe
        to repeat. The only thing it will not do for you is generate the SSH key."""
        self.cfg = cfg
        self.gitlab = cfg.gitlab # for convenience
        self.api = cfg.api
        self.request_until = cfg.request_until
        self.projects = []
        # Last submission this scenario produced, so a failure can show its report.
        self.submission_id = None

        # Without this GitLab refuses to deliver webhooks to the host.
        self.gitlab(
            "PUT",
            "/application/settings",
            {"allow_local_requests_from_web_hooks_and_services": True},
        )

        groups = self.gitlab("GET", f"/groups?search={urllib.parse.quote(cfg.namespace)}")
        group = next((g for g in groups if g["full_path"] == cfg.namespace), None)
        if group is None:
            group = self.gitlab("POST", "/groups", {"name": cfg.namespace, "path": cfg.namespace})
            print(f"created group {cfg.namespace}")

        # Student repositories live in this group, and a student has to be able to
        # push to main for anything to be graded. GitLab otherwise protects a new
        # project's default branch against pushes below maintainer.
        self.gitlab(
            "PUT",
            f"/groups/{group['id']}",
            {
                "default_branch_protection_defaults": {
                    "allowed_to_push": [{"access_level": 30}],
                    "allowed_to_merge": [{"access_level": 30}],
                    "allow_force_push": False,
                },
            },
        )

        found = self.gitlab("GET", f"/users?username={TEST_USER}")
        if found:
            user = found[0]
        else:
            user = self.gitlab(
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

        members = {
            m["id"]: m["access_level"]
            for m in self.gitlab("GET", f"/groups/{group['id']}/members?per_page=100")
        }
        # Maintainer, so that the autograder can seed a project with its first commit
        # and set commit statuses on the protected default branch. An existing member
        # is raised to the level it needs, since one left over at a lower level fails
        # in ways that are tedious to trace back to here.
        level = members.get(autograder["id"])
        if level != 40:
            verb, suffix = ("POST", "") if level is None else ("PUT", f"/{autograder['id']}")
            self.gitlab(
                verb,
                f"/groups/{group['id']}/members{suffix}",
                {"user_id": autograder["id"], "access_level": 40},
            )
            print(f"{autograder['username']} is now a maintainer of {cfg.namespace}")

        # Compare the key material, not the whole line, which carries a comment.
        pubkey_path = SSH_KEY.with_suffix(".pub")
        pubkey = pubkey_path.read_text().strip()
        registered = self.gitlab("GET", f"/users/{user['id']}/keys")
        if not any(k["key"].split()[1] == pubkey.split()[1] for k in registered):
            self.gitlab(
                "POST",
                f"/users/{user['id']}/keys",
                {"title": "autograder test suite", "key": pubkey},
            )
            print(f"registered {pubkey_path.name} with {TEST_USER}")

        self.group_id = group["id"]
        self.pusher_id = user["id"]

    def create_project(self, slug):
        """An empty project in the configured namespace, with a push webhook
        pointing at the autograder.."""
        name = f"{self.cfg.prefix}itest-{int(time.time())}-{slug}"
        project = self.gitlab(
            "POST",
            "/projects",
            {
                # `name` and `path` are identical on purpose: the submit handler drops
                # the push when the name from `path_with_namespace` differs from
                # `project.name`, and the calls back to GitLab address the project as
                # `<namespace>/<name>`.
                "name": name,
                "path": name,
                "namespace_id": self.group_id,
                "visibility": "private",
                "initialize_with_readme": False,
            },
        )
        self.projects.append(project)
        print(f"    created {project['path_with_namespace']}")

        # Seeded by the autograder, so that `main` exists before the pusher gets
        # near it. Done as the autograder rather than as admin to prove it holds
        # the access it needs on a real project.
        self.request_until(
            "gitlab",
            f"/projects/{project['id']}/repository/commits",
            method="POST",
            token=self.cfg.autograder_token,
            body={
                "branch": "main",
                "commit_message": "Initial commit",
                "actions": [
                    {"action": "create", "file_path": "README.md", "content": f"{name}\n"}
                ],
            },
            until=lambda status, _: status != 404,
        )

        # A student can push to its own repository and nothing else, so the pusher
        # is a member of this project alone, at a level that cannot administer it.
        self.gitlab(
            "POST",
            f"/projects/{project['id']}/members",
            {"user_id": self.pusher_id, "access_level": 30},
        )

        # Registered last, so that none of the setup above looks like a submission.
        self.gitlab(
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
            # A clone rather than a fresh history, since the project already
            # carries the commit the autograder seeded it with.
            git("clone", "-q", "-b", branch, project["ssh_url_to_repo"], tmp, cwd=tmp)

            for path, content in files.items():
                dest = Path(tmp) / path
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_bytes(content if isinstance(content, bytes) else content.encode())

            git("add", "-A", cwd=tmp)
            git(
                "-c", "user.name=itest",
                "-c", "user.email=itest@example.com",
                "commit", "-q", "-m", message,
                cwd=tmp,
            )
            git("push", "-q", "origin", branch, cwd=tmp)
            sha = git("rev-parse", "HEAD", cwd=tmp)

        print(f"    pushed {sha[:12]}: {message!r}")
        return sha

    def wait_for_submission(self, sha, timeout=120):
        """The submission the autograder registered for this commit, which is
        the direct evidence that the webhook arrived."""
        found = self.request_until(
            "autograder",
            f"/submission?source_kind=gitlab&commit_hash={sha}",
            until=lambda status, data: status == 200 and data["items"],
            timeout=timeout,
            interval=3,
        )["items"]
        self.submission_id = found[0]["submission_id"]
        print(f"    submission {self.submission_id}")
        return self.submission_id

    def wait_for_status(self, project, sha, timeout=600):
        """Blocks until GitLab carries a terminal commit status, which only
        happens once the autograder has reported back."""
        last = None

        def until(_status, data):
            nonlocal last
            if data:
                status = max(data, key=lambda s: s["id"])["status"]
                if status != last:
                    last = status
                    print(f"    status: {last}")
            return last in TERMINAL_STATUSES

        self.request_until(
            "gitlab",
            f"/projects/{project['id']}/repository/commits/{sha}/statuses",
            until=until,
            timeout=timeout,
            interval=5,
        )
        return last

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
        """Ends a scenario: drops its projects and forgets its state."""
        for project in self.projects:
            self.gitlab("DELETE", f"/projects/{project['id']}")
        self.projects = []
        self.submission_id = None
