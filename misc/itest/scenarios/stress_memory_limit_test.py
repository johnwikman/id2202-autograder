"""A solution that builds but exhausts its memory when a test runs it is stopped
as exceeding the memory limit, rather than reported as a failed test."""

from ..harness import files_from, register_scenario

TAG = "hello-limit-memory"


@register_scenario("direct-stresstest")
def run(ctx):
    files = files_from("misc/itest/files/memory-bomb-test", "solutions/hello")
    submission_id = ctx.direct.submit_ok(files, [TAG], entity="itest-memory-test")
    submission = ctx.direct.wait_for_grading(submission_id)

    jobs = {job["tag"]: job for job in submission["jobs"]}
    assert set(jobs) == {TAG}, sorted(jobs)
    status = jobs[TAG]["status"]
    # 483 = TestMemoryLimitExceeded
    assert status["code"] == 483, status
    assert status["successful"] is False, status
