# Tuist

[Tuist](https://tuist.dev) provides shared caching, live run reporting and
application previews for Once. Configure the capabilities your project needs
independently.

::: info Trigger orchestration is coming
Tuist support for [Once triggers](/guide/infrastructure/triggers) is coming.
This page covers caching, run reporting and application previews, not CI
orchestration.
:::

## Create And Connect A Project

Provision a Tuist project and bind this repository to it in one command:

```sh
once connect --provider tuist --create
```

Once signs you in when needed, creates the project under your personal account,
and writes the provider binding into the repository root `once.toml`. Read
[Connect A Project](/guide/infrastructure/connect) for account and project
selection. The result is the same configuration described below.

## Configure The Cache

Add a Tuist provider and cache binding to the repository root `once.toml`:

```toml
[infrastructures.tuist]
kind = "tuist"
account = "acme"
project = "app"

[infrastructure.cache]
provider = "tuist"
```

Replace `acme` and `app` with the Tuist account and project that should own the
shared entries.

## Authenticate A Developer Machine

Sign in once:

```sh
once auth login --provider tuist
```

Once reuses the stored session for cache reads, cache writes and preview
publication. Remove it when the machine should no longer access the provider:

```sh
once auth logout --provider tuist
```

## Verify A Shared Result

Use two machines with the same `once.toml`, script, and input. On the first
machine, run the [example scripted workflow](/guide/scripted/):

```sh
./scripts/greet.sh
```

The first run reports a cache miss. On a second machine whose local cache has
not seen this result, create the same `message.txt` and run the script again:

```sh
printf 'hello from Once\n' > message.txt
./scripts/greet.sh
```

The second machine should report a cache hit and restore
`build/greeting.txt`. Because its local cache has no matching entry, the hit
confirms that Tuist supplied the recorded result.

## Publish Application Previews

Configure Tuist as the artifact publication provider in the repository root
`once.toml`. If the project already defines this provider for caching, reuse
that definition and add only the artifact binding:

```toml
[infrastructures.tuist]
kind = "tuist"
account = "acme"
project = "application"

[infrastructure.artifacts]
name = "tuist"
```

```sh
once auth login --provider tuist
once artifact publish MyApplication --track review
```

`apple_application` supplies the bundle and native metadata when exporting on
macOS. Its `dev.once.apple-install.v1` interface describes the platform, SDK
variant, bundle identifier and minimum OS. Applications are a generic artifact
category; this interface describes installation specifically on Apple platforms.

The Tuist adapter uses the existing authentication flow, streams multipart
uploads, and returns a human preview URL only after completion. No executable
inspection or toolchain-specific build logic lives in the adapter. See
[Artifacts](/guide/infrastructure/artifacts) for the
provider-neutral export and publication model.

### Supported Formats And Limits

Tuist's current preview API supports app bundles, IPAs and APKs, not arbitrary
artifact manifests. The adapter requires one artifact with
`dev.tuist.preview.v1`, pointing to an upload payload and a
`spec.metadata_payload` containing the native request JSON. Custom producers
are responsible for making that metadata consistent with their payload.

This compatibility path uploads the selected payload, not the complete Once
manifest set. It does not promise atomic variant publication, custom retention
or retry-safe receipts. Duplicate builds fail explicitly, even if the previous
bytes have expired. An interrupted upload or unconfirmed completion is an
error; do not blindly retry. The URL identifies the preview, not permanent byte
availability. Generic Tuist artifact storage needs additional server support.

## Follow Runs Live

Once streams build and test runs to Tuist when the provider is configured and
the machine is authenticated. Each run publishes ordered event batches to the
project's account and project, and the service returns a dashboard link that
Once prints as the run starts. Open it to follow the graph, cache decisions,
and captured output while the run progresses.

Live reporting is best-effort: a missing endpoint or an expired session is
logged and never fails the run. Re-authenticate with `once auth login
--provider tuist` when the session expires.

## Authenticate Automation

Choose the authentication method supported by the environment.

### Account Token

Set `TUIST_TOKEN` to a Tuist account token authorized for the project's operations
when the runner can store a long-lived secret:

```sh
TUIST_TOKEN=tuist_... ./scripts/greet.sh
```

### OpenID Connect

Use [OpenID Connect](https://openid.net/developers/how-connect-works/) on a
supported automation runner. Log in without opening a browser before running
cacheable commands:

```sh
once auth login --provider tuist --no-browser
./scripts/greet.sh
```

On GitHub Actions, grant identity-token permission before the login step:

```yaml
permissions:
  id-token: write
  contents: read

steps:
  - uses: actions/checkout@v6
  - run: once auth login --provider tuist --no-browser
  - run: ./scripts/greet.sh
```

The resulting session remains available for the rest of the job.

## Connection recovery

Once retries transient cache failures during blob existence checks, uploads, and
action-result publication with at most three attempts per RPC. This includes
HTTP/2 connection retirement, where the cache asks the client to use a new
connection. Interrupted uploads restart from the beginning with a fresh upload
resource and a newly opened file or stream.

Authentication failures, permission refusals, invalid requests, quota refusals,
requests that exceed the request timeout, and local file errors are not
retried, so a cache that stops responding fails after one timeout rather than
three. Retries do not provide resumable uploads.

## Next

Add an [execution provider](/guide/infrastructure/remote-execution) when the
command should also run away from the current computer. Read
[Memory](/guide/memory/) to inspect the status, cache state, and action identity
that Once records for each run.
