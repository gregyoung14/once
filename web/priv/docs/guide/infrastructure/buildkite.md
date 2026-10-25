# Buildkite

Buildkite's [dynamic pipelines](https://buildkite.com/docs/pipelines/configure/dynamic-pipelines)
accept generated JSON steps. A small adapter can select Once triggers and
upload worker jobs without a repository pipeline YAML file.

Buildkite schedules workers and reports their status. Once owns the build
graph inside each worker. **Generate one job per matching trigger, not one
job per Once action.**

::: warning Example adapter, not a native integration
This example connects Buildkite to the experimental Once worker interface.
There is no installed Once Buildkite plugin. It covers branch-push verification
on one Linux x86-64 worker pool, not pull requests, tags, releases, or deployments.
:::

## Prepare The Service And Workers

Configure the pipeline to receive branch pushes only. Install the generator
below as `once-buildkite-pipeline` in your trusted bootstrap image, outside
the candidate checkout. Keep pipeline settings, hooks, the generator, and
worker setup outside candidate control.

Set `ONCE_DEFINITION_REVISION` in trusted service settings to the full lowercase
commit ID containing the approved triggers. Fetch that commit in every checkout.
Do not replace this value automatically with the commit being tested. Approve
trigger changes separately, including trigger retirement.

The bootstrap needs Once, Git, and Python 3. The `once-linux` queue needs Once,
Git, jq, a POSIX shell, the selected toolchains, and a clean checkout of the
build's full commit ID. Ignore `.once/` and keep temporary event files outside
the checkout. Use separate checkouts for concurrent jobs.

**Provide an isolated execution boundary before checkout or tool setup.** Do
not expose agent tokens, pipeline-upload credentials, secrets, or other jobs'
files to project code. Agent targeting and `env` entries are not isolation;
ordinary agent defaults do not establish this boundary. The example assumes
you already enforce it with your worker infrastructure and trusted hooks.

## Generate Worker Jobs From Approved Triggers

The adapter reads activated declarations without evaluating project modules.
It matches the source ref and carries the approved definition revision and
candidate commit into each generated job. It rejects unsupported build types,
incompatible configurations, and an empty selection rather than silently
reporting a successful verification.

```python
#!/usr/bin/env python3
import json
import os
import subprocess

if (os.environ.get("BUILDKITE_SOURCE") != "webhook"
        or os.environ.get("BUILDKITE_PULL_REQUEST") != "false"
        or os.environ.get("BUILDKITE_TAG")):
    raise SystemExit("This adapter accepts branch-push builds only")

approved = os.environ["ONCE_DEFINITION_REVISION"]
subprocess.run(
    ["once", "edit", "activate-triggers", "--revision", approved],
    check=True, stdout=subprocess.DEVNULL,
)
registered = json.loads(subprocess.check_output(
    ["once", "--format", "json", "query", "triggers"], text=True,
))["activation"]
ref = "refs/heads/" + os.environ["BUILDKITE_BRANCH"]
revision = os.environ["BUILDKITE_COMMIT"]
steps = []

for trigger in registered["triggers"]:
    if ref not in trigger["on"]["refs"]:
        continue
    if any((i["configuration"]["os"], i["configuration"]["arch"])
           != ("linux", "x86_64") for i in trigger["invoke"]):
        raise SystemExit("This queue supports Linux x86-64 configurations only")
    steps.append({
        "key": "once-" + trigger["id"],
        "label": "Once: " + trigger["id"],
        "agents": {"queue": "once-linux"},
        "timeout_in_minutes": 30,
        "env": {
            "ONCE_DEFINITION_REVISION": registered["revision"],
            "ONCE_REF": ref,
            "ONCE_REVISION": revision,
            "ONCE_TRIGGER": trigger["id"],
        },
        "command": [
            "set -eu",
            'once edit activate-triggers --revision "$ONCE_DEFINITION_REVISION"',
            'observation=$(mktemp)',
            "trap 'rm -f \"$observation\"' EXIT",
            'jq -n --arg ref "$ONCE_REF" --arg revision "$ONCE_REVISION" '
            "'{event: \"vcs.ref_changed\", ref: $ref, revision: $revision}' "
            '> "$observation"',
            'once --memory-limit 2GiB run --trigger "$ONCE_TRIGGER" '
            '--event "$observation"',
        ],
    })

if not steps:
    raise SystemExit("No approved Once trigger matches this ref")
print(json.dumps({"steps": steps}))
```

Set your Buildkite pipeline's bootstrap command in its service settings to:

```sh
set -eu
pipeline=$(mktemp)
trap 'rm -f "$pipeline"' EXIT
once-buildkite-pipeline > "$pipeline"
buildkite-agent pipeline upload --no-interpolation "$pipeline"
```

Writing the output before uploading ensures generator failure stops the
bootstrap instead of uploading partial work. `--no-interpolation` leaves
worker variables and jq expressions untouched until the worker runs. Stable
step keys make repeated uploads fail instead of adding duplicate jobs. Do
not automatically retry a bootstrap that may already have uploaded steps.

The approved trigger supplies the target selections and configurations.
The adapter supplies only service concerns: worker routing, timeout, and the
per-job memory budget. Adapt routing for additional worker pools; declaring
an OS in Once does not provision or isolate a machine with that OS.

## Validate Before Connecting A Real Pipeline

With a representative clean checkout and Buildkite environment, inspect the
generated pipeline without changing a running build:

```sh
once-buildkite-pipeline > /tmp/once-pipeline.json
buildkite-agent pipeline upload --no-interpolation --dry-run --format json \
  /tmp/once-pipeline.json
```

Dry-run validates the pipeline format, not agent placement, source trust,
required-check policy, or delivery to a real build. Test those in a disposable
Buildkite pipeline before adopting the adapter.

Once requires the worker's clean `HEAD` to match `ONCE_REVISION`. A missing
capability, failed invocation, dirty checkout, or mismatched revision fails
the job. Publish the aggregate build result through Buildkite; do not turn a
missing job or a crashed worker into success. Retain run records from `.once/`
through trusted infrastructure if you need them after the worker exits.

## Current Limits

Buildkite still owns event authentication, worker isolation, cancellation,
required checks, and recovery. The normalized observation is not an
authenticated event receipt. These runs use a dedicated local cache and do
not provide immutable replay. `--memory-limit` budgets work within one Once
process; enforce OS resource limits and limits across jobs separately.

See [Event-Driven Automation](/guide/infrastructure/triggers) for the trigger
contract and integration status of other services.
