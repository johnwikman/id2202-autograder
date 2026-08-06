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
    assert status != "canceled", "the autograder reported an internal failure, not a verdict"
    assert status == "success", f"expected success, got {status}"

    submission = ctx.api(f"/submission/{submission_id}")
    assert submission["finished"], "not marked finished"
    assert submission["successful"], "not marked successful"
    assert set(submission["grading_tags"]) == {"hello-asm", "hello-extra-more"}, submission["grading_tags"]
