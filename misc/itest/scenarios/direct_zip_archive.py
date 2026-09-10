"""A zip is unpacked and graded the same as a tar.gz, and is repacked into
the same storage format."""

from ..harness import files_from, register_scenario


@register_scenario("direct")
def run(ctx):
    files = files_from("misc/example-solutions/hello", "solutions/hello")
    submission_id = ctx.direct.submit_ok(
        files, ["hello"], entity="itest-zip", archive="zip"
    )
    submission = ctx.direct.wait_for_grading(submission_id)

    jobs = submission["jobs"]
    assert len(jobs) == 1, f"expected one job, got {[j['tag'] for j in jobs]}"
    assert jobs[0]["tag"] == "hello", jobs[0]["tag"]
    assert jobs[0]["status"]["code"] == 200, jobs[0]["status"]
    assert jobs[0]["status"]["successful"], jobs[0]["status"]
