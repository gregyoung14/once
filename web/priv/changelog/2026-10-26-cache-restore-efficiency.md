---
title: "Restore cached build outputs with less overhead"
date: 2026-10-26
---

Remote cache hits reuse the blob availability established by their lookup,
avoiding redundant requests. Small output downloads can share bounded batch
requests, while large blobs and servers without batching support keep streaming.

Read-only staging pins raw blobs with a file handle and avoids synchronizing
ephemeral decoded scratch files. Persistent cache contents and mutable output
copies retain their existing durability guarantees, and all staged content is
verified before it becomes readable.

Batch responses are structurally bounded before decoding. Stalled responses
time out, and cancelling one reader releases its resources without waiting for
other readers in the same batch.
