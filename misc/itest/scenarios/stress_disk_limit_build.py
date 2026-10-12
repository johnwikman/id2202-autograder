"""A solution that fills its disk while building is stopped as exceeding the disk
limit, rather than reported as a build error or an autograder failure."""

from ..harness import files_from, register_scenario

TAG = "hello-limit-disk"


@register_scenario("direct-stresstest")
def run(ctx):
    files = files_from("misc/itest/files/disk-bomb-build", "solutions/hello")
    submission_id = ctx.direct.submit_ok(files, [TAG], entity="itest-disk-build")
    submission = ctx.direct.wait_for_grading(submission_id)

    jobs = {job["tag"]: job for job in submission["jobs"]}
    assert set(jobs) == {TAG}, sorted(jobs)
    status = jobs[TAG]["status"]
    # 474 = BuildDiskSpaceLimitExceeded
    assert status["code"] == 474, status
    assert status["successful"] is False, status
