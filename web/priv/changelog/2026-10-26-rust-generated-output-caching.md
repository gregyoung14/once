---
title: Cache Rust compilation by generated output content
date: 2026-10-26
---

Rust compilation can now reuse cached artifacts after a generator rebuilds
without changing its generated files or compiler directives, provided every
other compiler input is unchanged.

Build-script output directories also participate in downstream cache keys and
input materialization. Changing a generated native library now relinks its
consumers even when the Rust sources and build-script directives stay the
same. Restoring a cached build includes those generated artifacts.

Build scripts keep conservative package-wide input tracking. Generated files
that contain changing timestamps still invalidate their consumers.
