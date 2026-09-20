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
    """Passes the test case.

    Exits the program with code 0, does not return."""
    write_verdict(True, None)


def reject(reason: str) -> NoReturn:
    """Fails the test case, showing `reason` to the student.

    Exits the program with code 0, does not return."""
    write_verdict(False, reason)


def expect(condition: bool, reason: str) -> None:
    """Fails the test case if `condition` if False, in which case it rejects
    with the specified reason.

    This is a short-hand for `reject` wrapped in an if-statement:

    ```py
    if not (x > 5):
        reject("x is not greater than 5")

    # Is equivalent to
    expect(x > 5, "x is not greater than 5")
    ```
    """
    if not condition:
        reject(reason)


@contextmanager
def no_except(reason: str) -> Iterator[None]:
    """Turns any exception raised inside the block into a `reject` with the
    specified reason.

    Example:

    ```py
    with no_except("expected UTF-8"):
        # These function will raise an exception if the encoded data is not
        # UTF-8 encoded. As such, an exception here would be an issue with
        # the graded program.
        stdout = run.stdout.as_utf8()
        stderr = run.stderr.as_utf8()
    ```
    """
    try:
        yield
    except Exception:
        reject(reason)
