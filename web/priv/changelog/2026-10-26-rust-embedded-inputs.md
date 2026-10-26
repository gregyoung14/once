---
title: Correct cache invalidation for embedded Rust inputs
date: 2026-10-26
---

Rust compilation now fingerprints every file declared by `srcs`, including
non-Rust files embedded at compile time. Changing an embedded template, schema,
or other declared data file rebuilds the crate and its affected consumers
instead of restoring an outdated artifact from cache.
