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
    assert status != "canceled", "the autograder reported an internal failure, not a verdict"
    assert status == "success", f"expected success, got {status}"

    submission = ctx.api(f"/submission/{submission_id}")
    assert submission["finished"], "not marked finished"
    assert submission["successful"], "not marked successful"
    assert submission["grading_tags"] == ["hello-all"], submission["grading_tags"]
