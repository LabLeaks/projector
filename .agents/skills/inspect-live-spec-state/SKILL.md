---
name: inspect-live-spec-state
description: Use this skill when you need the current live validated product-contract state for projector. Materialize the live tree with `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs`, then scope into exact claims with `... special specs SPEC.ID --verbose`.
---

# Inspect Live Spec State

Use this skill when you need to understand what `projector` currently claims is live and supported.

1. Start with `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs` to view the live tree only.
2. If you need a narrower view, scope to the exact node with `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs SPEC.ID`.
3. If you need to understand why a claim is live, use `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs SPEC.ID --verbose`.
4. Treat `@group` nodes as navigation only; the real contract lives on direct `@spec` nodes.
5. Use this skill before making claims about what `projector` currently ships.

Read [references/state-walkthrough.md](references/state-walkthrough.md) for the walkthrough and [references/trigger-evals.md](references/trigger-evals.md) for trigger examples.
