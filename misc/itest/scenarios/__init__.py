"""The scenarios, in the order they run."""

from . import (
    direct_invalid_payload,
    direct_unknown_tag,
    direct_archive_too_large,
    direct_sink_acceptance,
    direct_sink_grading_report,
    direct_hello_all,
    direct_zip_archive,
    stress_payload_limits,
    stress_throttling,
    stress_resubmission_storm,
    stress_concurrent_submitters,
    gitlab_build_failure,
    gitlab_hello_all,
    gitlab_multiple_tags,
    gitlab_superseded,
    gitlab_test_failure,
    gitlab_unknown_tag,
)
