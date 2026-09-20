"""Helpers for writing verifier programs.

A verifier receives one JSON object describing a single execution of a student
binary on stdin, and writes a verdict as JSON on stdout. This package holds the
I/O boilerplate and the type annotations for both ends of that exchange.

Note about terminology:

 * `assert` means the verifier (or the autograder config) itself is broken,
   which aborts grading procedure for the entire tag.
 * `expect` means the student is wrong, which fails this test case, but
   proceeds to grade the next test case.

Example usage:

```py
import autograder_verifier_tools as avt

run = avt.read_stdin()

with avt.no_except("output is not UTF-8 encoded integer string"):
    x = int(run.stdout.as_utf8().strip())

if x < 0:
    avt.reject("output is negative")

d = run.params["divisor"]
assert isinstance(d, int), "error with param config for divisor in autograder"

avt.expect(x % d == 0, f"output is not divisible by {d}")
avt.accept()
```
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
