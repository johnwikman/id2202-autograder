"""A solution that does not build is reported as failed, not as an autograder error."""


def run(ctx):
    project = ctx.create_project("build-failure")
    files = ctx.files_from("misc/test_gitlab/files/build-failure", "solutions/hello")
    sha = ctx.push(project, files, "#hello")

    submission_id = ctx.wait_for_submission(sha)
    status = ctx.wait_for_status(project, sha)
    # The point of this scenario: "canceled" would mean the autograder blamed
    # itself for the student's build error.
    assert status == "failed", f"expected failed, got {status}"

    submission = ctx.api(f"/submission/{submission_id}")
    # A build error belongs to the tag that failed to build, not to the
    # submission as a whole.
    assert submission["report"] is None, f"unexpected submission report: {submission['report']}"

    jobs = {job["tag"]: job for job in submission["jobs"]}
    assert set(jobs) == {"hello"}, sorted(jobs)
    job = jobs["hello"]

    # 470 = BuildError, should not see any other error
    assert job["status"]["code"] == 470, job["status"]
    assert job["status"]["successful"] is False, job["status"]

    report = job["report"]
    assert report is not None, "no report on the failed job"
    assert "tag_grading" in report, f"expected a tag_grading report, got {list(report)}"
    assert report["tag_grading"]["build_failure"] is not None, f"no build failure recorded: {report}"
