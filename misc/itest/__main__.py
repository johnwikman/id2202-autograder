"""Runs the scenarios against a running stack. Nothing here starts anything.

Direct submissions need only postgres and the autograder:

    sudo docker compose up -d postgres
    export AUTOGRADER_SERVER_API_AUTH_TOKENS="example-api-token"
    ./target/debug/entrypoint -s example/settings.toml start
    python3 -m misc.itest direct

GitLab scenarios additionally need a running GitLab, and push over SSH as a
dedicated user which the suite creates and grants access itself. The one thing
it will not do for you is generate the key it pushes with:

    ssh-keygen -t ed25519 -N "" -f data/ssh/itest_ed25519

    sudo docker compose up -d postgres gitlab
    export AUTOGRADER_RUNNER_SSH_KEYS="$(pwd)/data/ssh/itest_ed25519"
    AUTOGRADER_SERVER_ADDRESS=0.0.0.0 dotenv run --override \
        ./target/debug/entrypoint -s example/settings.toml start
    python3 -m misc.itest gitlab

`full` runs everything, and is the default. A scenario is a function in
scenarios/ decorated with `@register_scenario(<features>)`; it fails by
raising, so plain asserts are enough.
"""

import argparse
import json
import sys
import time
import tomllib
import traceback
from pathlib import Path

from . import harness
from .harness import FEATURES, REGISTERED_SCENARIOS
from .harness.direct import DirectConfig, DirectContext
from .harness.gitlab import GitLabConfig, GitLabContext


def main():
    parser = argparse.ArgumentParser(
        prog="python3 -m misc.itest",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "features",
        nargs="*",
        default=["full"],
        choices=["full", *FEATURES],
        help="which submission origins to exercise (default: full)",
    )
    parser.add_argument(
        "-k",
        "--scenario",
        action="append",
        choices=[*REGISTERED_SCENARIOS],
        help="run only this scenario; repeatable (default: all that match)",
    )
    parser.add_argument(
        "-s",
        "--settings",
        default=harness.REPO_ROOT / "example" / "settings.toml",
        help="settings file the autograder was started with",
    )
    args = parser.parse_args()

    selected = set(FEATURES) if "full" in args.features else set(args.features)
    scenarios = [
        s
        for name, s in REGISTERED_SCENARIOS.items()
        if set(s.feats) & selected and (not args.scenario or name in args.scenario)
    ]
    if not scenarios:
        raise SystemExit(f"no scenarios match {sorted(selected)}")

    settings = tomllib.loads(Path(args.settings).read_text())
    autograder = harness.Autograder.load(settings)
    # Only what the selected scenarios need: building a GitLabContext stands up
    # groups and users, and its config refuses to load without a live GitLab.
    needed = {feat for s in scenarios for feat in s.feats} & selected
    ctx = harness.Context(
        autograder=autograder,
        direct=(
            DirectContext(DirectConfig.load(settings, autograder))
            if "direct" in needed
            else None
        ),
        gitlab=(
            GitLabContext(GitLabConfig.load(settings, autograder))
            if "gitlab" in needed
            else None
        ),
    )

    failures = []
    for scenario in scenarios:
        print(f"{scenario.name}: {scenario.doc}", flush=True)
        started = time.monotonic()
        error = None
        try:
            scenario.run(ctx)
        except Exception:
            failures.append(scenario.name)
            error = traceback.format_exc().rstrip()
            if ctx.submission_id:
                report = autograder.get(f"/submission/{ctx.submission_id}").get("report")
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
