# Change Workflow

Use this checklist when shipping a change that should keep the product contract honest:

1. Identify the exact behavior that changed.
2. If the claim does not exist yet, add it before or alongside the implementation.
3. Keep the claim present tense and narrow enough that one verify can honestly support it.
4. Use `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs SPEC.ID --verbose` to inspect the existing support before changing code.
5. Tighten weak verifies until a reviewer can judge the claim locally.
6. If the work is not ready to ship, keep the claim planned instead of pretending it is live.
7. Re-run `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs` and `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special lint` after the change to confirm the tree still materializes cleanly.
