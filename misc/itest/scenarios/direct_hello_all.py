"""The reference solutions, submitted as a tar.gz, pass every tag in the
hello-all group."""

from ..harness import files_from, register_scenario

TAGS = ("hello", "hello-asm", "hello-extra", "hello-file")


@register_scenario("direct")
def run(ctx):
    files = {}
    for tag in TAGS:
        files |= files_from(f"misc/example-solutions/{tag}", f"solutions/{tag}")

    submission_id = ctx.direct.submit_ok(files, ["hello-all"], entity="itest-hello-all")
    submission = ctx.direct.wait_for_grading(submission_id)

    assert submission["requested_tags"] == ["hello-all"], submission["requested_tags"]
    assert submission["report"] is None, f"unexpected submission report: {submission['report']}"
    assert submission["origin"] == {
        "direct": {"domain": ctx.direct.cfg.domain, "entity": "itest-hello-all"}
    }, submission["origin"]

    # The group expands into one job per tag, each recording the name that was
    # actually requested.
    jobs = {job["tag"]: job for job in submission["jobs"]}
    assert set(jobs) == set(TAGS), f"expected a job per tag, got {sorted(jobs)}"
    for tag, job in jobs.items():
        assert job["requested_as"] == ["hello-all"], (tag, job["requested_as"])
        assert job["status"]["code"] == 200, (tag, job["status"])
        assert job["status"]["successful"], (tag, job["status"])
        assert job["report"] is not None, f"{tag} finished without a report"
        assert job["started_at"] is not None, f"{tag} has no start time"
        assert job["finished_at"] is not None, f"{tag} has no finish time"
