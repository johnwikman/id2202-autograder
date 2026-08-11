"""Helpers for writing verifier programs.

A verifier receives one JSON object describing a single execution of a student
binary on stdin, and writes a verdict as JSON on stdout. This package holds the
I/O boilerplate and the type annotations for both ends of that exchange.

`assert` means the verifier itself is broken, which aborts grading. `expect`
means the student is wrong, which fails the test case. Keep the two distinct.
"""

from .grading import accept, expect, no_except, reject
from .interface import Encoded, ParamValue, ProtocolError, Run, read_stdin

__all__ = [
    "Encoded",
    "ParamValue",
    "ProtocolError",
    "Run",
    "accept",
    "expect",
    "no_except",
    "read_stdin",
    "reject",
]
