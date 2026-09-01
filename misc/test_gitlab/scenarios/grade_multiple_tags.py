"""Check that it accepts multiple grading tags."""

SOLUTION_DIRS = ("hello-asm", "hello-extra")


def run(ctx):
    project = ctx.create_project("multiple-tags")

    files = {}
    for dir in SOLUTION_DIRS:
        files |= ctx.files_from(f"misc/example-solutions/{dir}", f"solutions/{dir}")
    sha = ctx.push(project, files, "#hello-asm #hello-extra-more")

    submission_id = ctx.wait_for_submission(sha)
    status = ctx.wait_for_status(project, sha, timeout=900)
    assert status != "canceled", "nothing was graded at all: rejected tags, or every job voided"
    assert status == "success", f"expected success, got {status}"

    submission = ctx.api(f"/submission/{submission_id}")
    requested = set(submission["requested_tags"])
    assert requested == {"hello-asm", "hello-extra-more"}, requested
    assert submission["report"] is None, f"unexpected submission report: {submission['report']}"

    # Two tags named directly, so each job is requested as itself rather than
    # through a group.
    jobs = {job["tag"]: job for job in submission["jobs"]}
    assert set(jobs) == {"hello-asm", "hello-extra-more"}, sorted(jobs)
    for tag, job in jobs.items():
        assert job["requested_as"] == [tag], (tag, job["requested_as"])
        assert job["status"]["successful"], (tag, job["status"])
        assert job["eligible_at"] is None, f"{tag} was throttled unexpectedly"
        assert job["voided_at"] is None, f"{tag} was voided unexpectedly"
