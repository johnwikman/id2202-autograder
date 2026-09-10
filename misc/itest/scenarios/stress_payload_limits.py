"""Payloads at and beyond the configured limits are answered rather than
crashing the server or hanging it."""

from ..harness import register_scenario

# `submission.max_payload` is 64 KiB, so this is comfortably past it even
# before the JSON envelope.
OVERSIZE_PAYLOAD = "A" * 200_000

# `is_valid_entity` allows 1..=60 characters.
LONG_ENTITY = "e" * 61

# `submission.max_tag_length` is 128 characters, measured over the tags
# together.
LONG_TAGS = [f"tag-{n:03}-aaaaaaaaaaaaaaaaaaaa" for n in range(20)]


@register_scenario("direct-stresstest")
def run(ctx):
    # Refused outright: the request never becomes a submission.
    for label, expected, kwargs in [
        ("payload past max_payload", 400, {"data": OVERSIZE_PAYLOAD}),
        ("entity of 61 characters", 400, {"entity": LONG_ENTITY, "data": "e30="}),
    ]:
        status, parsed = ctx.direct.submit({}, ["hello"], **kwargs)
        assert status == expected, f"{label}: expected {expected}, got {status} {parsed}"

    # Recorded, but with nothing to grade and a report saying why.
    submission_id = ctx.direct.submit_ok(
        {"solutions/hello/main.c": "int main(void){return 0;}\n"},
        LONG_TAGS,
        entity="itest-stress-tags",
    )
    submission = ctx.autograder.get(f"/submission/{submission_id}")
    assert submission["jobs"] == [], f"expected no jobs, got {submission['jobs']}"
    assert submission["report"] is not None, "over-long tags produced no report"

    # Well inside every size limit but awkward to unpack, so the server has to
    # answer one way or the other rather than fall over.
    for label, files in [
        ("4000 tiny files", {f"solutions/hello/f{n:04}.txt": b"x" for n in range(4000)}),
        ("a path 80 levels deep", {"solutions/hello/" + "d/" * 80 + "f.txt": b"x"}),
    ]:
        status, parsed = ctx.direct.submit(files, ["hello"], entity="itest-stress-shape")
        assert status in (201, 400), f"{label}: expected 201 or 400, got {status} {parsed}"
        print(f"    {label}: {status}")
