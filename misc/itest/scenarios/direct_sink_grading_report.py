"""A submitter with a sink is sent the grading results when they are ready,
so it never has to poll for the outcome."""

from ..harness import files_from, register_scenario

ENTITY = "itest-sink-report"


@register_scenario("direct")
def run(ctx):
    sink = ctx.direct.sink()
    files = files_from("misc/example-solutions/hello", "solutions/hello")
    submission_id = ctx.direct.submit_ok(files, ["hello"], entity=ENTITY, sink=sink)
    ctx.direct.wait_for_grading(submission_id)

    # The sink sits at a fixed address, so a submission from an earlier run
    # that this one supersedes still reports its own outcome here. Acceptance
    # and claim messages arrive first, so match on the terminal pair rather
    # than on a number of messages.
    mine = lambda m: m.body["submission_id"] == submission_id
    state = sink.wait_for("state", where=lambda m: mine(m) and m.body["state"] == "success")[0]
    report = sink.wait_for(
        "report", where=lambda m: mine(m) and "job_results" in m.body["report"]
    )[0]

    for message in (state, report):
        assert message.signature_ok, (
            f"{message.message_type} carried a bad HMAC: {message.signature}"
        )
        assert message.body["submission_id"] == submission_id, message.body
        assert message.body["entity"] == ENTITY, message.body

    jobs = report.body["report"]["job_results"]
    assert len(jobs) == 1, f"expected one job in the report, got {len(jobs)}"
    job = jobs[0]
    assert job["tag"] == "hello", job["tag"]
    assert job["status"] == "Success", job["status"]
    assert job["finished_at"] is not None, job
    # The grading result itself, which is what the submitter is actually after.
    assert "tag_grading" in job["report"], f"expected a grading report, got {list(job['report'])}"
    assert job["report"]["tag_grading"]["tag_name"] == "hello", job["report"]["tag_grading"]

    # Everything the submitter needed arrived by push, so the sequence stands
    # on its own without the polling endpoint.
    states = [m.body["state"] for m in sink.of_type("state", mine)]
    assert states[0] == "waiting", states
    assert states[-1] == "success", states
    print(f"    states pushed: {states}")
