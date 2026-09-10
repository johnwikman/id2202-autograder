"""
Grading utilities for a verifier.

`accept` and `reject` ends the verification process immediately, exiting with
code 0. The remaining function may conditionally exit the verification process.
"""

from collections.abc import Iterator
from contextlib import contextmanager
from typing import NoReturn

from .interface import write_verdict


def accept() -> NoReturn:
    """Passes the test case. Exits the program with code 0, does not return."""
    write_verdict(True, None)


def reject(reason: str) -> NoReturn:
    """Fails the test case, showing `reason` to the student. Exits the program
    with code 0, does not return."""
    write_verdict(False, reason)


def expect(condition: bool, reason: str) -> None:
    """Fails the test case if `condition` if False, in which case it rejects
    with the specified reason."""
    if not condition:
        reject(reason)


@contextmanager
def no_except(reason: str) -> Iterator[None]:
    """Turns any exception raised inside the block into a `reject` with the
    specified reason."""
    try:
        yield
    except Exception:
        reject(reason)
