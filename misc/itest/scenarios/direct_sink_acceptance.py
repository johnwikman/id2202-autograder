"""A submitter that provides a sink is told the submission was accepted,
over a signed callback, without having to poll."""

from ..harness import files_from, register_scenario

ENTITY = "itest-sink"


@register_scenario("direct")
def run(ctx):
    sink = ctx.direct.sink()
    files = files_from("misc/example-solutions/hello", "solutions/hello")
    submission_id = ctx.direct.submit_ok(files, ["hello"], entity=ENTITY, sink=sink)

    # The submit handler sets the state and sends the acceptance report
    # concurrently, so both land without a runner ever claiming the job.
    state = sink.wait_for("state")[0]
    report = sink.wait_for("report")[0]

    for message in (state, report):
        assert message.signature_ok, (
            f"{message.message_type} carried a bad HMAC: {message.signature}"
        )
        assert message.body["submission_id"] == submission_id, message.body
        assert message.body["domain"] == ctx.direct.cfg.domain, message.body
        assert message.body["entity"] == ENTITY, message.body

    assert state.body["state"] == "waiting", state.body
    assert state.body["description"] == "Waiting In Queue", state.body
    # The acceptance message is prose rather than a grading result, so it
    # arrives as structured text.
    assert "structured" in report.body["report"], report.body["report"]
    assert str(submission_id) in report.body["report"]["structured"], report.body["report"]
