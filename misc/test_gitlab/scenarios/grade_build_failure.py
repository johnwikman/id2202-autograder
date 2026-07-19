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
    assert not submission["successful"], "a failing build was reported as successful"

    # Anything that goes wrong before grading starts — a failed clone, say —
    # also ends as "failed", so check that the build is what actually broke.
    report = submission["report"]
    assert "submission" in report, f"grading never started: {report}"
    tags = report["submission"]["tag_reports"]
    assert any(t["build_failure"] for t in tags), f"no build failure reported: {tags}"
