"""A solution that builds but fills its disk when a test runs it is stopped as
exceeding the disk limit, rather than reported as a failed test."""

from ..harness import files_from, register_scenario

TAG = "hello-limit-disk"


@register_scenario("direct-stresstest")
def run(ctx):
    files = files_from("misc/itest/files/disk-bomb-test", "solutions/hello")
    submission_id = ctx.direct.submit_ok(files, [TAG], entity="itest-disk-test")
    submission = ctx.direct.wait_for_grading(submission_id)

    jobs = {job["tag"]: job for job in submission["jobs"]}
    assert set(jobs) == {TAG}, sorted(jobs)
    status = jobs[TAG]["status"]
    # 484 = TestDiskSpaceLimitExceeded
    assert status["code"] == 484, status
    assert status["successful"] is False, status
