---
name: validate-product-contract
description: Use this skill when checking whether a projector product claim is honestly supported. Inspect the exact claim with `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs SPEC.ID --verbose`, read the attached support, and decide whether the contract and verification truly match.
---

# Validate Product Contract

Use this skill when you need to judge whether a live `projector` product claim is honestly supported.

1. Start from one exact claim, not a whole subtree.
2. Run `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs SPEC.ID --verbose` and read the claim text before reading the attached support.
3. Review each attached `@verifies` or `@attests` body and decide whether it proves the exact claim rather than a nearby one.
4. If the verify hides important setup behind helpers, treat that as a quality problem and tighten it.
5. If the claim is not actually ready to ship, move it back to `@planned` instead of pretending the support is good enough.
6. Run `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special lint` if the support looks malformed or the references seem inconsistent.

Read [references/validation-checklist.md](references/validation-checklist.md) for the review rubric and [references/trigger-evals.md](references/trigger-evals.md) for trigger examples.
