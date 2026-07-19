"""An unknown grading tag is accepted by the server and rejected by the runner."""

TAG = "definitely-not-a-tag"


def run(ctx):
    project = ctx.create_project("unknown-tag")
    sha = ctx.push(project, {"README.md": "Nothing to build here.\n"}, f"#{TAG}")

    # The submit handler only validates the length of the tags, so this push is
    # registered and it falls to the runner to notice that no such tag exists.
    submission_id = ctx.wait_for_submission(sha)
    status = ctx.wait_for_status(project, sha, timeout=300)
    assert status == "failed", f"expected failed, got {status}"

    submission = ctx.api(f"/submission/{submission_id}")
    assert not submission["successful"], "an unknown tag was reported as successful"

    report = submission["report"]["wrapper"]["reports"][0]
    assert "invalid_tag" in report, f"expected an invalid_tag report, got {list(report)}"
    assert report["invalid_tag"]["tag_name"] == TAG, report["invalid_tag"]["tag_name"]
