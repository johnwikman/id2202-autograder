"""Runs the GitLab scenarios against a running stack. Nothing here starts
anything: bring up postgres, gitlab and the autograder first.

    sudo docker compose up -d postgres gitlab
    export AUTOGRADER_SERVER_API_AUTH_TOKENS="itest-token"
    export AUTOGRADER_RUNNER_SSH_KEYS="$(pwd)/data/ssh/itest_ed25519"
    AUTOGRADER_SERVER_ADDRESS=0.0.0.0 dotenv run --override \
        ./target/debug/entrypoint -s example/settings.toml start

The push goes over SSH as a dedicated GitLab user, which the suite creates and
grants access itself. The one thing it will not do for you is generate the key
it pushes with:

    ssh-keygen -t ed25519 -N "" -f data/ssh/itest_ed25519

A scenario is a module in scenarios/ defining run(ctx); it fails by raising, so
plain asserts are enough.
"""

import argparse
import json
import sys
import time
import traceback

from . import harness
from .scenarios import SCENARIOS


def main():
    names = {module.__name__.rsplit(".", 1)[-1]: module for module in SCENARIOS}

    parser = argparse.ArgumentParser(
        prog="python3 -m misc.test_gitlab",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "scenario",
        nargs="*",
        choices=[*names],
        help="scenarios to run (default: all of them)",
    )
    parser.add_argument(
        "-s",
        "--settings",
        default=harness.REPO_ROOT / "example" / "settings.toml",
        help="settings file the autograder was started with",
    )
    args = parser.parse_args()

    cfg = harness.Config.load(args.settings)
    ctx = harness.Context(cfg)

    failures = []
    for name in args.scenario or names:
        module = names[name]
        print(f"{name}: {module.__doc__.strip()}", flush=True)
        started = time.monotonic()
        error = None
        try:
            module.run(ctx)
        except Exception:
            failures.append(name)
            error = traceback.format_exc().rstrip()
            if ctx.submission_id:
                report = ctx.api(f"/submission/{ctx.submission_id}").get("report")
                error += f"\n\nreport for submission {ctx.submission_id}:\n"
                error += json.dumps(report, indent=2)
        finally:
            ctx.cleanup()

        print(f"  {'FAIL' if error else 'pass'} ({time.monotonic() - started:.1f}s)", flush=True)
        if error:
            print("\n".join(f"  {line}" for line in error.splitlines()), flush=True)
        print(flush=True)

    print(f"{len(failures)} failed" if failures else "all passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
