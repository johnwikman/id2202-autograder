"""One submitter resubmitting the same tag over and over leaves every job in a
terminal state, and the newest submission is the one that survives."""

from ..harness import files_from, register_scenario

ENTITY = "itest-stress-storm"
ROUNDS = 8

# Graded, superseded by a newer submission, or refused because the tag's budget
# is spent. A job in any other state means one of these was dropped.
ACCOUNTED_FOR = (200, 409, 429)


@register_scenario("direct-stresstest")
def run(ctx):
    files = files_from("misc/example-solutions/hello", "solutions/hello")

    ids = []
    for n in range(ROUNDS):
        status, parsed = ctx.direct.submit(files, ["hello"], entity=ENTITY)
        assert status == 201, f"round {n} was rejected: {status} {parsed}"
        ids.append(parsed["submission_id"])
    print(f"    {ROUNDS} submissions from one entity: {min(ids)}..{max(ids)}")

    outcomes = {}
    for submission_id in ids:
        ctx.direct.submission_id = submission_id
        submission = ctx.direct.wait_for_grading(submission_id)
        for job in submission["jobs"]:
            assert job["status"]["finished"], (submission_id, job["status"])
            assert job["status"]["code"] in ACCOUNTED_FOR, (submission_id, job["status"])
            outcomes[submission_id] = job["status"]["code"]
    print(f"    outcomes: {sorted(outcomes.values())}")

    # Superseding only replaces what a runner has not claimed, so the newest
    # submission has nothing behind it that could void it.
    for job in ctx.autograder.get(f"/submission/{ids[-1]}")["jobs"]:
        assert job["voided_at"] is None, f"the newest submission was voided: {job}"
        assert job["status"]["code"] != 409, f"the newest submission was superseded: {job}"
