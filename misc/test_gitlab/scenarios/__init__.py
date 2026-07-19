"""The scenarios, in the order they run."""

from . import grade_build_failure, grade_hello_all, grade_unknown_tag

SCENARIOS = (grade_hello_all, grade_unknown_tag, grade_build_failure)
