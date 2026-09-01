"""Pushing the same tag again replaces the earlier run while it is still queued."""

BLOCKING_TAGS = ("hello", "hello-asm", "hello-extra", "hello-file")

# Superseding only touches jobs no runner has claimed, so this first occupies the
# runner with a four-tag submission. A source with a job being graded is excluded
# from claims, so the two pushes behind it stay pending and the second replaces
# the first.
#
# Note that this assumes that we manage to submit the two subsequent "#hello"
# jobs before the initial #hello-all job has finished. May need to run this
# multiple times to double-check in case nothing was superseded.
def run(ctx):
    project = ctx.create_project("supersede")

    files = {}
    for tag in BLOCKING_TAGS:
        files |= ctx.files_from(f"misc/example-solutions/{tag}", f"solutions/{tag}")

    # Intentionally slow down the "hello" build so the race this scenario
    # depends on has a higher chance of happening.
    files["solutions/hello/Makefile"] = files["solutions/hello/Makefile"].replace(
        b"all:\n", b"all:\n\tsleep 15\n"
    )

    # Every push has to change something, or git refuses to commit. The
    # solutions stay identical; only this marker moves.
    def attempt(n):
        return files | {"attempt.txt": f"{n}\n"}

    # Holds the source for as long as it takes to grade four tags.
    blocking = ctx.push(project, attempt(0), "#hello-all")
    ctx.wait_for_submission(blocking)

    # Queued behind it, so nothing claims this before the next push lands.
    first = ctx.push(project, attempt(1), "#hello")
    first_id = ctx.wait_for_submission(first)

    second = ctx.push(project, attempt(2), "#hello")
    second_id = ctx.wait_for_submission(second)

    replaced = ctx.api(f"/submission/{first_id}")
    jobs = {job["tag"]: job for job in replaced["jobs"]}
    job = jobs["hello"]
    assert job["status"]["code"] == 409, (
        f"expected the queued job to be superseded, got {job['status']}."
        " A runner may have claimed it before the second push landed."
    )
    assert job["voided_at"] is not None, "a superseded job should be voided"
    assert job["status"]["successful"] is False, job["status"]

    # The replacement is an ordinary job of the newer submission, untouched.
    replacement = ctx.api(f"/submission/{second_id}")
    new_jobs = {job["tag"]: job for job in replacement["jobs"]}
    assert new_jobs["hello"]["voided_at"] is None, "the replacement was voided"

    # Nothing will grade the replaced submission, so the submit path has to be
    # what closes its commit out. Every job of it was voided, which is what
    # GitLab calls cancelled.
    status = ctx.wait_for_status(project, first, timeout=180)
    assert status == "canceled", f"expected canceled, got {status}"

    comments = "\n".join(ctx.commit_comments(project, first))
    assert "replaced by a newer submission" in comments, f"never told it was replaced: {comments}"
    assert f"\\(ID {second_id}\\)" in comments, f"the replacement is not named: {comments}"

    # Leave nothing in flight for the scenarios that follow.
    ctx.wait_for_status(project, blocking, timeout=900)
    ctx.wait_for_status(project, second, timeout=900)
