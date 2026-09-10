"""A malformed direct submission is turned away by the submit handler, and
nothing is recorded for it."""

from ..harness import register_scenario

# Valid base64 that is not an archive, for the checks that run before the
# archive is ever unpacked.
NOT_AN_ARCHIVE = "e30="


@register_scenario("direct")
def run(ctx):
    for label, expected, kwargs in [
        ("bad secret", 401, {"secret": "not-the-secret", "data": NOT_AN_ARCHIVE}),
        ("unknown domain", 401, {"domain": "nowhere.invalid", "data": NOT_AN_ARCHIVE}),
        ("entity with a slash", 400, {"entity": "itest/nested", "data": NOT_AN_ARCHIVE}),
        ("empty entity", 400, {"entity": "", "data": NOT_AN_ARCHIVE}),
        ("unsupported archive", 400, {"archive": "rar", "data": NOT_AN_ARCHIVE}),
        ("unsupported encoding", 400, {"encoding": "rot13", "data": NOT_AN_ARCHIVE}),
        ("data that is not base64", 400, {"data": "!!! not base64 !!!"}),
        ("a tar.gz that is not one", 400, {"data": NOT_AN_ARCHIVE}),
    ]:
        status, parsed = ctx.direct.submit({}, ["hello"], **kwargs)
        assert status == expected, f"{label}: expected {expected}, got {status} {parsed}"

    # Tags are checked before anything is written, so an empty list is refused
    # rather than recorded as a submission with nothing to do.
    status, parsed = ctx.direct.submit({}, [], data=NOT_AN_ARCHIVE)
    assert status == 400, f"no grading tags: expected 400, got {status} {parsed}"
