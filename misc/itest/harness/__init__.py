"""Support code shared by every origin the suite can submit through."""

import json
import os
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

REPO_ROOT = Path(__file__).resolve().parents[3]

# Every feature a scenario may be registered under. "full" is the CLI's word
# for all of them and is never a feature itself.
FEATURES = ("direct", "gitlab")


@dataclass
class Scenario:
    run: Callable
    name: str
    doc: str
    feats: list[str]


REGISTERED_SCENARIOS: dict[str, Scenario] = {}


def register_scenario(*feats):
    """Registers the run function of a scenario for testing.

    The scenario is named after the module it is defined in, and described by
    its own docstring or, lacking one, that module's.
    """
    caller = sys._getframe(1).f_globals
    name = caller["__name__"].rsplit(".", 1)[-1]

    def apply(f):
        REGISTERED_SCENARIOS[name] = Scenario(
            run=f,
            name=name,
            doc=(f.__doc__ or caller.get("__doc__") or "").strip(),
            feats=list(feats),
        )
        return f

    return apply


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
        raise AssertionError(f"could not reach {url}: {e.reason}")
    try:
        return status, json.loads(raw)
    except ValueError:
        return status, raw.decode(errors="replace")


def poll(call, *, until, what, timeout=60, interval=0.5):
    """Repeats `call()`, which returns (status, parsed), until
    `until(status, parsed)` holds, and returns the parsed body.

    Fails on timeout, or if the call that satisfied `until` was not a 2xx.
    `what` names the call in those messages.
    """
    deadline = time.monotonic() + timeout
    status, parsed = None, None
    while time.monotonic() < deadline:
        status, parsed = call()
        if until(status, parsed):
            if not 200 <= status < 300:
                raise AssertionError(f"{what}: {status} {parsed}")
            return parsed
        time.sleep(interval)
    raise AssertionError(f"{what} still {status} after {timeout}s: {parsed}")


def files_from(source, prefix):
    """A directory tree below `source`, relative to the repository root, as
    a {path: bytes} mapping rooted at `prefix`.

    Fails if `source` holds no files, which would otherwise submit an empty
    tree and assert nothing.
    """
    root = REPO_ROOT / source
    if not any(p.is_file() for p in root.rglob("*")):
        raise AssertionError(f"no fixture files under {source}")
    return {
        f"{prefix}/{p.relative_to(root)}": p.read_bytes()
        for p in sorted(root.rglob("*"))
        if p.is_file()
    }


@dataclass
class Autograder:
    """The autograder's own REST API, which every origin asserts against."""

    api: str
    token: str

    @classmethod
    def load(cls, settings):
        api_tokens = settings["server"]["secrets"]["api_auth_tokens"]
        token = os.environ.get("AUTOGRADER_SERVER_API_AUTH_TOKENS", "").split(";")[0] or (
            api_tokens[0] if api_tokens else ""
        )
        if not token:
            raise SystemExit(
                "No autograder API token. Export AUTOGRADER_SERVER_API_AUTH_TOKENS for "
                "both the autograder and this shell."
            )
        port = int(os.environ.get("AUTOGRADER_SERVER_PORT", settings["server"]["port"]))
        address = os.environ.get("AUTOGRADER_SERVER_ADDRESS", settings["server"]["address"])
        host = "127.0.0.1" if address == "0.0.0.0" else address
        return cls(api=f"http://{host}:{port}/api", token=token)

    def request(self, path, method="GET", body=None):
        """Returns (status, parsed)."""
        return http(
            method,
            f"{self.api}{path}",
            headers={"Authorization": f"Bearer {self.token}"},
            body=body,
        )

    def get(self, path):
        """A GET that is expected to succeed."""
        status, parsed = self.request(path)
        assert status == 200, f"GET {path}: {status} {parsed}"
        return parsed

    def until(self, path, *, until, timeout=60, interval=0.5):
        """A GET repeated until `until(status, parsed)` holds."""
        return poll(
            lambda: self.request(path),
            until=until,
            what=f"autograder GET {path}",
            timeout=timeout,
            interval=interval,
        )


@dataclass
class Context:
    """What a scenario is handed. Only the origins it was registered under are
    set; the others are `None`."""

    autograder: Autograder
    direct: "object | None" = None
    gitlab: "object | None" = None

    @property
    def submission_id(self):
        """The last submission any origin registered, for a failure to report
        against."""
        for origin in (self.direct, self.gitlab):
            if origin is not None and origin.submission_id is not None:
                return origin.submission_id
        return None

    def cleanup(self):
        for origin in (self.direct, self.gitlab):
            if origin is not None:
                origin.cleanup()
