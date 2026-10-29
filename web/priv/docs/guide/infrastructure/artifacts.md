# Artifacts

**Artifacts are immutable build products. Publishing sends them to your
configured infrastructure.** Once builds, packages and publishes the content;
the provider decides how people download, install, serve or run it.

An application, command-line program, server, static site, dataset or report
can be an artifact. No container runtime or running web server is required.

## Package explicitly selected content

```toml
[[target]]
name = "site"
kind = "artifact"
srcs = ["public/**/*"]

[target.attrs.interfaces."dev.once.static-http.v1"]
payload = "content"

[target.attrs.interfaces."dev.once.static-http.v1".spec]
root = "public"
index = "index.html"
```

`artifact` creates a deterministic ZIP with package-relative paths. Its
`file_mode` defaults to 420 (0644); use 493 (0755) for executable files.
Permissions are declared, not inferred from the packaging host. Custom target
kinds can export existing file outputs directly or declare mixed-mode archives.

```sh
once query artifacts site
once artifact export site --output dist
once artifact publish site --provider local
```

Export returns the immutable root descriptor and a self-contained directory.
Local publication retains the package separately from the action cache and
writes a publication receipt after all content is present. Repeating it repairs
missing or changed content. Deleting the workspace still deletes local
publications; use an external export destination for longer retention.

## Content describes what it is

Each artifact has named payloads, each with a SHA-256 digest, byte size and
media type. A manifest binds payloads to optional versioned interfaces. An
artifact set groups named variants with explicit selectors. Neither target
paths, Git revisions, tracks, credentials nor infrastructure URLs are part of
an individual artifact's content identity.

Exported packages contain `index.json` and `blobs/sha256/<digest>` files.
The root describes an `once.artifact_set.v1` document; members refer to
`once.artifact.v1` manifests, which refer to payloads. JSON object keys use
stable ordering and compact UTF-8 serialization. Consumers verify every
descriptor's digest and size before trusting content.

An interface has `payload` and `spec` fields. Unknown interfaces survive
export and remain downloadable; they never grant permission to execute a
command or dynamically load a handler. Examples of consumption contracts:

- **Download:** No interface required. Retain arbitrary bytes.
- **Static site:** `dev.once.static-http.v1` declares the archive `root` and
  `index`. Never serve private sibling files.
- **CLI:** A producer-owned executable contract declares entrypoint, arguments,
  OS, architecture and required runtime content.
- **Server:** A producer-owned process contract adds ports, readiness and
  runtime requirements. Secrets and assigned addresses come from infrastructure.
- **Application:** A producer-owned installation contract declares the package,
  platform, runtime compatibility and installation requirements.

Consumption interfaces are declarations. Once does not implement static hosting,
CLI installation or server activation. An executable alone is not a complete
runtime closure. A compatible consumer must validate its supported contract,
unpack safely, isolate untrusted content, bind secrets with authorization,
allocate resources, and own routing, access, retention and cleanup.

## Choose publication infrastructure

```toml
[infrastructure.artifacts]
name = "local"
```

`artifacts` is the capability being configured; `name` selects its provider.
The built-in `local` provider needs no separate definition. Named provider
definitions can be reused by the capabilities they support.

Cache, execution and artifact bindings are independent. Publication is explicit,
never skipped because the build was cached, and does not run the published content.
An absent artifact binding is an error unless `--provider` selects one.
`--track` adds distribution context without rebuilding different content.

Provider-specific configuration, authentication, supported formats and limits
live in each provider's guide. See [Tuist](/guide/infrastructure/tuist#publish-application-previews)
to publish supported applications to Tuist.

## Use the same surface from an agent

- `once_query_artifacts` inspects capability support and the provider shape.
- `once_export_artifacts` mirrors local export.
- `once_publish_artifacts` mirrors explicit publication.

The last two require `once mcp --allow-run`. Authorize the selected destination
before asking an agent to publish. See the [module contract](/reference/modules/#artifact-providers)
to add artifact support to another target kind.
