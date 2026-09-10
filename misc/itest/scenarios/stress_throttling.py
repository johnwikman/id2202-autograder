"""A tag past its rate limit is held back rather than refused, and a tag past
its budget is refused outright."""

import time
from datetime import datetime

from ..harness import files_from, register_scenario

# `hello-throttle-rate` allows 2 per hour, `hello-throttle-budget` allows 3 in
# total. Both count per submitter and neither forgets within a run, so each
# run needs a submitter of its own.
RATE_N = 2
BUDGET_MAX_RUNS = 3


@register_scenario("direct-stresstest")
def run(ctx):
    files = files_from("misc/example-solutions/hello", "solutions/hello")
    entity = f"itest-throttle-{int(time.time())}"

    # Each run has to finish before the next is submitted. A job still waiting
    # to be claimed is superseded by the next submission of the same tag, and
    # a superseded job deliberately does not count towards either limit.
    def graded(tag):
        submission_id = ctx.direct.submit_ok(files, [tag], entity=entity)
        return ctx.direct.wait_for_grading(submission_id)

    # Rate limited: still accepted, but parked until the oldest run in the
    # window ages out of it.
    for n in range(RATE_N):
        job = graded("hello-throttle-rate")["jobs"][0]
        assert job["status"]["code"] == 200, f"run {n} did not grade: {job['status']}"

    submission_id = ctx.direct.submit_ok(files, ["hello-throttle-rate"], entity=entity)
    submission = ctx.autograder.get(f"/submission/{submission_id}")
    job = submission["jobs"][0]
    assert job["status"]["code"] == 0, f"expected a queued job, got {job['status']}"
    held = datetime.fromisoformat(job["eligible_at"]) - datetime.fromisoformat(
        submission["submitted_at"]
    )
    assert held.total_seconds() > 60, f"the run past the limit was only held {held}"
    print(f"    rate limit: run {RATE_N + 1} held back {held}")

    # Budgeted: the run past the budget is rejected outright, so nothing will
    # ever grade it.
    for n in range(BUDGET_MAX_RUNS):
        job = graded("hello-throttle-budget")["jobs"][0]
        assert job["status"]["code"] == 200, f"run {n} did not grade: {job['status']}"

    submission_id = ctx.direct.submit_ok(files, ["hello-throttle-budget"], entity=entity)
    job = ctx.autograder.get(f"/submission/{submission_id}")["jobs"][0]
    assert job["status"]["code"] == 429, f"expected the run past the budget to be rejected: {job}"
    assert job["status"]["finished"], job["status"]
    print(f"    budget: run {BUDGET_MAX_RUNS + 1} rejected")
