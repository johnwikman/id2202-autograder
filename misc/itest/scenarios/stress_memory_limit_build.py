"""A solution that exhausts its memory while building is stopped as exceeding the
memory limit, rather than reported as a build error or an autograder failure."""

from ..harness import files_from, register_scenario

TAG = "hello-limit-memory"


@register_scenario("direct-stresstest")
def run(ctx):
    files = files_from("misc/itest/files/memory-bomb-build", "solutions/hello")
    submission_id = ctx.direct.submit_ok(files, [TAG], entity="itest-memory-build")
    submission = ctx.direct.wait_for_grading(submission_id)

    jobs = {job["tag"]: job for job in submission["jobs"]}
    assert set(jobs) == {TAG}, sorted(jobs)
    status = jobs[TAG]["status"]
    # 473 = BuildMemoryLimitExceeded
    assert status["code"] == 473, status
    assert status["successful"] is False, status
