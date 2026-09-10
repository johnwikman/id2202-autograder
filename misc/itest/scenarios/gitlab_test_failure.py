"""A solution that builds but prints the wrong thing fails its test cases, and
is recorded as a different status than one that fails to build."""



from ..harness import files_from, register_scenario

@register_scenario("gitlab")
def run(ctx):
    project = ctx.gitlab.create_project("test-failure")
    files = files_from("misc/itest/files/wrong-output", "solutions/hello")
    sha = ctx.gitlab.push(project, files, "#hello")

    submission_id = ctx.gitlab.wait_for_submission(sha)
    status = ctx.gitlab.wait_for_status(project, sha)
    assert status == "failed", f"expected failed, got {status}"

    submission = ctx.autograder.get(f"/submission/{submission_id}")
    assert submission["report"] is None, f"unexpected submission report: {submission['report']}"

    jobs = {job["tag"]: job for job in submission["jobs"]}
    assert set(jobs) == {"hello"}, sorted(jobs)
    job = jobs["hello"]

    # 480 = TestCasesFailed
    assert job["status"]["code"] == 480, job["status"]
    assert job["status"]["successful"] is False, job["status"]

    report = job["report"]["tag_grading"]
    assert report["build_failure"] is None, f"blamed the build: {report['build_failure']}"
    assert not report["ok"], "a tag with a failing test reported itself ok"
    assert len(report["groups"]) > 0, "no test groups reported"
