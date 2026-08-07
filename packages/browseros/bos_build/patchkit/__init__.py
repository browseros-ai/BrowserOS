"""Python patch surface: dev extract + pipeline batch-apply + .features.yaml IO.

Interactive patch workflows (apply/sync/conflicts) live in the Rust tool
(tools/bpatch, `bpatch`); the build pipeline must never depend on it, so
non-interactive batch apply stays here in Python by design.
"""
