---
name: ship-product-change
description: Use this skill when adding a feature, fixing a bug, or changing behavior in projector that should update the product contract. Define or revise the relevant product specs first, then keep each live claim matched to one honest verify.
---

# Ship Product Change

Use this skill when a `projector` product change needs to stay aligned with the contract you ship.

1. Start from the user-visible or system-visible behavior that changed, not the implementation.
2. Find the relevant claim with `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs` or `... special specs --all`.
3. If the behavior is not ready to ship, keep the exact claim `@planned` instead of over-claiming.
4. When the claim is live, make sure it has one honest, self-contained `@verifies` or `@attests` artifact.
5. Run `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs SPEC.ID --verbose` before trusting support. Read the attached body and decide whether it actually proves the claim.
6. If the change affects command behavior, prefer command-boundary verifies over helper-only tests.
7. Run `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special lint` after spec edits.

Read [references/change-workflow.md](references/change-workflow.md) for the detailed workflow and [references/trigger-evals.md](references/trigger-evals.md) for trigger examples.
