"""What crosses the wire between the autograder and a verifier.

The autograder writes one JSON object on stdin describing a single execution of
the student's program; the verifier writes one JSON object on stdout saying
whether it accepts. Reading and writing live together because they are two
halves of the same contract, and changing one without the other breaks it.

What arrives is validated rather than trusted. Input that does not match what
the autograder promised to send raises `ProtocolError`, which aborts grading:
it is an autograder bug, and must never be reported as a verdict about the
student.
"""

import base64
import json
import sys
from dataclasses import dataclass
from typing import NoReturn, cast

ParamValue = bool | int | str


class ProtocolError(Exception):
    """What arrived on stdin was not what the autograder promised to send.
    Always an autograder bug, never a student one."""


@dataclass(frozen=True)
class Encoded:
    """A byte string from the run. Tagged on the wire, so a verifier cannot
    silently handle only the encoding its author happened to see."""

    raw: bytes

    def as_bytes(self) -> bytes:
        return self.raw

    def is_utf8(self) -> bool:
        try:
            self.raw.decode("utf-8")
        except UnicodeDecodeError:
            return False
        return True

    def as_utf8(self) -> str:
        """Raises `UnicodeDecodeError` if the bytes are not valid UTF-8."""
        return self.raw.decode("utf-8")


@dataclass(frozen=True)
class Run:
    """One execution of the student's program."""

    cmd: list[str]
    code: int
    stdout: Encoded
    stderr: Encoded
    files: dict[str, Encoded]
    params: dict[str, ParamValue]


def _object(value: object, where: str) -> dict[str, object]:
    if not isinstance(value, dict):
        raise ProtocolError(f"{where}: expected an object")
    return cast("dict[str, object]", value)


def _encoded(value: object, where: str) -> Encoded:
    obj = _object(value, where)
    data = obj.get("data")
    if not isinstance(data, str):
        raise ProtocolError(f'{where}: "data" must be a string')
    match obj.get("enc"):
        case "utf8":
            return Encoded(data.encode("utf-8"))
        case "base64":
            return Encoded(base64.b64decode(data, validate=True))
        case enc:
            raise ProtocolError(f"{where}: unknown encoding {enc!r}")


def read_stdin() -> Run:
    """Reads and validates what the autograder writes on stdin."""
    incoming = _object(json.load(sys.stdin), "input")

    cmd = incoming.get("cmd")
    if not isinstance(cmd, list) or not all(
        isinstance(arg, str) for arg in cast("list[object]", cmd)
    ):
        raise ProtocolError('input: "cmd" must be a list of strings')

    # `bool` is a subclass of `int`, and an exit code is never a boolean.
    code = incoming.get("code")
    if not isinstance(code, int) or isinstance(code, bool):
        raise ProtocolError('input: "code" must be an integer')

    files = {
        name: _encoded(value, f'input: files["{name}"]')
        for name, value in _object(incoming.get("files"), 'input: "files"').items()
    }

    params: dict[str, ParamValue] = {}
    for name, value in _object(incoming.get("params"), 'input: "params"').items():
        if not isinstance(value, (bool, int, str)):
            raise ProtocolError(f'input: params["{name}"] must be a bool, int or str')
        params[name] = value

    return Run(
        cmd=cast("list[str]", cmd),
        code=code,
        stdout=_encoded(incoming.get("stdout"), 'input: "stdout"'),
        stderr=_encoded(incoming.get("stderr"), 'input: "stderr"'),
        files=files,
        params=params,
    )


def write_verdict(accepted: bool, reason: str | None) -> NoReturn:
    """Writes the verdict and exits, so nothing after it runs."""
    verdict: dict[str, object] = {"accepted": accepted}
    if reason is not None:
        verdict["reason"] = reason
    json.dump(verdict, sys.stdout)
    sys.stdout.flush()
    sys.exit(0)
