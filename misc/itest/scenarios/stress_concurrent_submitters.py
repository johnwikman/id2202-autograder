"""Many submitters arriving at once are each registered and each graded, with
nothing lost, duplicated or left unfinished."""

from concurrent.futures import ThreadPoolExecutor

from ..harness import files_from, register_scenario

SUBMITTERS = 12


@register_scenario("direct-stresstest")
def run(ctx):
    files = files_from("misc/example-solutions/hello", "solutions/hello")

    # Distinct entities, so none of these supersede one another and every one
    # of them has to be graded on its own.
    with ThreadPoolExecutor(max_workers=SUBMITTERS) as pool:
        results = list(
            pool.map(
                lambda n: ctx.direct.submit(files, ["hello"], entity=f"itest-stress-{n:02}"),
                range(SUBMITTERS),
            )
        )

    ids = []
    for n, (status, parsed) in enumerate(results):
        assert status == 201, f"submitter {n} was rejected: {status} {parsed}"
        ids.append(parsed["submission_id"])
    assert len(set(ids)) == SUBMITTERS, f"submission ids are not distinct: {sorted(ids)}"
    print(f"    {SUBMITTERS} submissions accepted: {min(ids)}..{max(ids)}")

    for submission_id in ids:
        ctx.direct.submission_id = submission_id
        submission = ctx.direct.wait_for_grading(submission_id)
        jobs = submission["jobs"]
        assert len(jobs) == 1, f"{submission_id}: expected one job, got {len(jobs)}"
        assert jobs[0]["status"]["code"] == 200, (submission_id, jobs[0]["status"])
        # Each grading gets its own container and workspace, so a collision
        # between two runners surfaces here as a job that failed for a reason
        # unrelated to the solution.
        assert jobs[0]["report"] is not None, f"{submission_id} finished without a report"
