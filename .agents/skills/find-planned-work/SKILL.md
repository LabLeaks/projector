---
name: find-planned-work
description: Use this skill when looking for product-contract work that is planned but not live yet. Materialize the full tree with `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs --all`, then focus on `[planned]` claims.
---

# Find Planned Work

Use this skill when you need to see what `projector` intends to ship later but has not made live yet.

1. Start with `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs --all` so planned claims appear.
2. Focus on `[planned]` nodes rather than live ones.
3. If the tree is large, scope with `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs --all SPEC.ID`.
4. Treat planned claims as backlog/roadmap inputs, not current behavior.
5. Use this skill for backlog discovery, v0/v1 triage, and release-readiness questions.

Read [references/planned-workflow.md](references/planned-workflow.md) for the walkthrough and [references/trigger-evals.md](references/trigger-evals.md) for trigger examples.
