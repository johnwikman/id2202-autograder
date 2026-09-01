"""The reference solutions pass every tag in the hello-all group."""

TAGS = ("hello", "hello-asm", "hello-extra", "hello-file")


def run(ctx):
    project = ctx.create_project("hello-all")

    files = {}
    for tag in TAGS:
        files |= ctx.files_from(f"misc/example-solutions/{tag}", f"solutions/{tag}")
    sha = ctx.push(project, files, "#hello-all")

    submission_id = ctx.wait_for_submission(sha)
    status = ctx.wait_for_status(project, sha, timeout=900)
    assert status != "canceled", "nothing was graded at all: rejected tags, or every job voided"
    assert status == "success", f"expected success, got {status}"

    submission = ctx.api(f"/submission/{submission_id}")
    assert submission["requested_tags"] == ["hello-all"], submission["requested_tags"]
    assert submission["report"] is None, f"unexpected submission report: {submission['report']}"

    # The group expands into one job per tag, each recording the name that was
    # actually written in the commit message.
    jobs = {job["tag"]: job for job in submission["jobs"]}
    assert set(jobs) == set(TAGS), f"expected a job per tag, got {sorted(jobs)}"
    for tag, job in jobs.items():
        assert job["requested_as"] == ["hello-all"], (tag, job["requested_as"])
        assert job["status"]["code"] == 200, (tag, job["status"])
        assert job["status"]["successful"], (tag, job["status"])
        assert job["status"]["finished"], (tag, job["status"])
        assert job["report"] is not None, f"{tag} finished without a report"
        assert job["started_at"] is not None, f"{tag} has no start time"
        assert job["finished_at"] is not None, f"{tag} has no finish time"

    # The tags of one claim are graded one after another, so they cannot all
    # have started at the same instant. They would if the claim stamped them.
    starts = {job["started_at"] for job in jobs.values()}
    assert len(starts) > 1, f"every job claims the same start time: {starts}"

    # What the student is told, in order: accepted, being graded, results.
    comments = "\n".join(ctx.commit_comments(project, sha))
    assert f"[Submission ID: {submission_id}" in comments, "no acceptance comment"
    assert "The autograder is now grading your submission." in comments, "no claim comment"
    assert "rate-limited" not in comments, f"nothing was throttled here: {comments}"
    assert "Submission Results" in comments, "no results comment"
    for tag in TAGS:
        assert f"Results for tag `{tag}`" in comments, f"{tag} missing from the results"
