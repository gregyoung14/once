---
title: Safer exit policies and cache reuse
date: 2026-10-13
---

Commands now fail when they exit with `0` but their `success_exit_codes` list
excludes it. Accepted nonzero exits continue to count as success, and failures
retain their captured diagnostics. After upgrading, affected actions run again
rather than reusing results recorded under the earlier behavior.

Cache validation no longer trusts mutable timestamps where reliable change
markers are unavailable. Content-based action caches continue to work, while
unsafe metadata-only analysis and build shortcuts are disabled on those
platforms. Rebuild checks on Windows may take longer as a result.

Very deep build graphs no longer risk stack exhaustion while scheduling. Action plans
validate dependencies in linear time, tolerate repeated dependency entries,
and report worker panics instead of waiting indefinitely. SDK action keys keep
their existing format while hashing inputs without buffering a second copy.

Garbage collection remains a best-effort filesystem snapshot. Concurrent
publication can cause a cache miss, which execution safely repairs by running
the action again. Files disappearing during a collection pass no longer make
the scan fail.
