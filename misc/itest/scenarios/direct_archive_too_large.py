"""An archive that is small on the wire but expands past
`submission.direct.max_unpacked_size` is refused, and nothing is stored."""

from ..harness import register_scenario


@register_scenario("direct")
def run(ctx):
    limit = ctx.direct.cfg.max_unpacked_size

    # Compresses to a few kilobytes, so the request itself stays well inside
    # `submission.max_payload` and only the unpacked size is out of bounds.
    bomb = {"zeros.bin": b"\0" * (limit + 1)}
    status, parsed = ctx.direct.submit(bomb, ["hello"], entity="itest-too-large")
    assert status == 400, f"expected the archive to be refused, got {status} {parsed}"

    # Just under it, the same shape is accepted, so the rejection above is the
    # size and not the content.
    ok = {"solutions/hello/zeros.bin": b"\0" * (limit // 2)}
    ctx.direct.submit_ok(ok, ["hello"], entity="itest-too-large")
