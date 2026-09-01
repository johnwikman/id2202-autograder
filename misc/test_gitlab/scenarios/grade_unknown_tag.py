"""An unknown grading tag is rejected at submit time, so the submission is
recorded with a report and no jobs and never reaches a runner."""

TAG = "definitely-not-a-tag"


def run(ctx):
    project = ctx.create_project("unknown-tag")
    sha = ctx.push(project, {"README.md": "Nothing to build here.\n"}, f"#{TAG}")

    submission_id = ctx.wait_for_submission(sha)
    # The submit handler resolves the tags, so this fails without waiting for a
    # runner to pick anything up.
    status = ctx.wait_for_status(project, sha, timeout=120)
    assert status == "failed", f"expected failed, got {status}"

    submission = ctx.api(f"/submission/{submission_id}")
    assert submission["requested_tags"] == [TAG], submission["requested_tags"]
    # Nothing about it was gradable, so there is no job to carry the failure
    # and it sits on the submission instead.
    assert submission["jobs"] == [], f"expected no jobs, got {submission['jobs']}"

    report = submission["report"]
    assert report is not None, "the submission carries no report"
    assert "invalid_tag" in report, f"expected an invalid_tag report, got {list(report)}"
    assert report["invalid_tag"]["tag_name"] == TAG, report["invalid_tag"]["tag_name"]
    assert len(report["invalid_tag"]["known_grading_tags"]) > 0, "the student is offered no alternatives"
