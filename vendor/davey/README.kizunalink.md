# KizunaLink davey patch

This directory vendors `davey` 0.1.4 from upstream commit
`a1e2e741bea06bc3b7167a5c3792844b8975993c`.

KizunaLink adds transition-aware outbound ratchet staging. Discord requires
receivers to prepare the next epoch before acknowledging a transition, while
senders must keep using the active epoch until voice gateway opcode 22 executes
the transition. Upstream 0.1.4 updates both directions immediately when an MLS
welcome or commit is processed, so the application cannot satisfy both parts of
that contract without this patch.

The upstream source remains MIT licensed; see `LICENSE`.
