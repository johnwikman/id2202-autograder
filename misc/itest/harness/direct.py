"""Direct submissions: an archive posted straight to the autograder.

Needs no GitLab, only postgres and a running autograder. Sinks listen on the
loopback, so they are only reachable if the autograder runs on the host rather
than inside a container.
"""

import base64
import hashlib
import hmac
import io
import json
import os
import tarfile
import threading
import time
import zipfile
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from . import Autograder, http

# Terminal for a submission as a whole: every job it has is finished.
GRADED_TIMEOUT = 900


def pack(files, archive="tar.gz"):
    """`files` ({path: bytes | str}) as the bytes of a `tar.gz` or `zip`
    archive."""
    buf = io.BytesIO()
    if archive == "zip":
        with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
            for path, content in sorted(files.items()):
                z.writestr(path, content if isinstance(content, bytes) else content.encode())
    elif archive in ("tar.gz", "targz"):
        with tarfile.open(fileobj=buf, mode="w:gz") as t:
            for path, content in sorted(files.items()):
                data = content if isinstance(content, bytes) else content.encode()
                info = tarfile.TarInfo(path)
                info.size = len(data)
                info.mtime = 0
                t.addfile(info, io.BytesIO(data))
    else:
        raise ValueError(f"unknown archive format {archive!r}")
    return buf.getvalue()


@dataclass
class SinkMessage:
    """One delivery the autograder made to a sink."""

    body: dict
    signature: str
    signature_ok: bool

    @property
    def message_type(self):
        return self.body.get("message_type")


class Sink:
    """An HTTP endpoint on the loopback that collects what the autograder
    posts to it, checking the HMAC of every delivery as it arrives."""

    def __init__(self, secret_key):
        self.secret_key = secret_key
        self.messages = []
        self._lock = threading.Lock()
        sink = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def do_POST(self):
                raw = self.rfile.read(int(self.headers.get("Content-Length", 0)))
                sink._record(self.headers.get("X-HMAC-Signature", ""), raw)
                self.send_response(200)
                self.send_header("Content-Length", "0")
                self.end_headers()

            def log_message(self, *_args):
                pass

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    @property
    def url(self):
        host, port = self._server.server_address[:2]
        return f"http://{host}:{port}/"

    @property
    def spec(self):
        """The `sink` object of a direct submission payload."""
        return {"url": self.url, "secret_key": self.secret_key}

    def _record(self, signature, raw):
        expected = "sha256=" + hmac.new(
            self.secret_key.encode(), raw, hashlib.sha256
        ).hexdigest()
        try:
            body = json.loads(raw)
        except ValueError:
            body = {"unparsed": raw.decode(errors="replace")}
        with self._lock:
            self.messages.append(
                SinkMessage(
                    body=body,
                    signature=signature,
                    signature_ok=hmac.compare_digest(signature, expected),
                )
            )

    def of_type(self, message_type):
        with self._lock:
            return [m for m in self.messages if m.message_type == message_type]

    def wait_for(self, message_type, count=1, timeout=120):
        """Blocks until `count` messages of `message_type` have arrived, and
        returns all of them."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            found = self.of_type(message_type)
            if len(found) >= count:
                return found
            time.sleep(0.2)
        with self._lock:
            seen = [m.message_type for m in self.messages]
        raise AssertionError(
            f"sink got {len(self.of_type(message_type))} {message_type!r} messages, "
            f"wanted {count}, after {timeout}s. Everything it did get: {seen}"
        )

    def close(self):
        self._server.shutdown()
        self._server.server_close()


@dataclass
class DirectConfig:
    autograder: Autograder
    domain: str
    secret: str
    max_unpacked_size: int

    @classmethod
    def load(cls, settings, autograder):
        direct = settings["submission"]["direct"]
        domains = direct["allowed_domains"]
        if not domains:
            raise SystemExit("submission.direct.allowed_domains is empty")
        return cls(
            autograder=autograder,
            domain=os.environ.get(
                "AUTOGRADER_SUBMISSION_DIRECT_ALLOWED_DOMAINS", ""
            ).split(";")[0] or domains[0],
            secret=os.environ.get("AUTOGRADER_SUBMISSION_DIRECT_SECRET")
            or direct["secret"],
            max_unpacked_size=int(
                os.environ.get(
                    "AUTOGRADER_SUBMISSION_DIRECT_MAX_UNPACKED_SIZE",
                    direct["max_unpacked_size"],
                )
            ),
        )


class DirectContext:
    def __init__(self, cfg: DirectConfig):
        self.cfg = cfg
        self.sinks = []
        # Last submission this scenario produced, so a failure can show its report.
        self.submission_id = None

    def sink(self, secret_key="itest-sink-key"):
        """A sink that is torn down when the scenario ends."""
        s = Sink(secret_key)
        self.sinks.append(s)
        return s

    def submit(
        self,
        files,
        tags,
        *,
        entity="itest",
        archive="tar.gz",
        encoding="base64",
        sink=None,
        domain=None,
        secret=None,
        data=None,
    ):
        """Posts `files` as a submission of `tags`, returning (status, parsed).

        `data` replaces the encoded archive outright, for submitting something
        that is not a valid one. Every other keyword overrides the field of the
        same name in the payload, so that a scenario can send a request the
        server should reject.
        """
        if data is None:
            data = base64.b64encode(pack(files, archive)).decode()
        payload = {
            "domain": self.cfg.domain if domain is None else domain,
            "entity": entity,
            "grading_tags": list(tags),
            "archive": archive,
            "encoding": encoding,
            "data": data,
        }
        if sink is not None:
            payload["sink"] = sink.spec if isinstance(sink, Sink) else sink
        return http(
            "POST",
            f"{self.cfg.autograder.api}/submit/direct",
            headers={"X-Direct-Secret": self.cfg.secret if secret is None else secret},
            body=payload,
        )

    def submit_ok(self, files, tags, **kwargs):
        """A submission that is expected to be accepted, returning its id."""
        status, parsed = self.submit(files, tags, **kwargs)
        assert status == 201, f"submit rejected: {status} {parsed}"
        self.submission_id = parsed["submission_id"]
        print(f"    submission {self.submission_id}: {parsed['message']}")
        return self.submission_id

    def wait_for_grading(self, submission_id, timeout=GRADED_TIMEOUT):
        """The submission once every one of its jobs has finished. One with no
        jobs was rejected at submit time and is already done."""
        last = None

        def until(status, data):
            nonlocal last
            if status != 200 or not isinstance(data, dict):
                return False
            states = [job["status"]["text"] for job in data["jobs"]]
            if states != last:
                last = states
                print(f"    jobs: {states or '(none)'}")
            return all(job["status"]["finished"] for job in data["jobs"])

        return self.cfg.autograder.until(
            f"/submission/{submission_id}",
            until=until,
            timeout=timeout,
            interval=3,
        )

    def cleanup(self):
        """Ends a scenario: stops its sinks and forgets its state."""
        for s in self.sinks:
            s.close()
        self.sinks = []
        self.submission_id = None
