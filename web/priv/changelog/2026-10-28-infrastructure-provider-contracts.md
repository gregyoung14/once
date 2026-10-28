---
title: Infrastructure provider integration guide and contracts
date: 2026-10-28
---

Infrastructure teams can now start with a top-to-bottom
[provider integration guide](/docs/guide/infrastructure/providers) and use
separate [cache](/docs/reference/infrastructure/cache) and
[execution](/docs/reference/infrastructure/execution) references while
implementing their service adapters.

The references cover content and action-result formats, the existing REAPI
cache transport, command preparation, resource limits, tenant isolation, and
machine ownership and cleanup. They link to current implementations and
regression tests and include acceptance checks for remote reuse and failures.

The documentation distinguishes contracts available today from adapter work
required for a new service. It does not introduce a new provider plug-in API
or add support for another hosted service.
