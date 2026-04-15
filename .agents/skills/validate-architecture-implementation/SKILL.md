---
name: validate-architecture-implementation
description: Use this skill when checking whether a concrete projector architecture module is honestly implemented by the code that claims to implement it. Inspect one module with `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special modules MODULE.ID --verbose`, use `--metrics` when you need unknown-unknowns evidence, and decide whether the module intent and implementation really match.
---

# Validate Architecture Implementation

Use this skill when you need to judge whether a concrete `projector` architecture module is honestly implemented by the code that claims to implement it.

1. Start from one exact `@module` id, not a whole subtree.
2. Run `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special modules MODULE.ID --verbose` and read the module text before reading the attached `@implements` or `@fileimplements` bodies.
3. Use `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special modules MODULE.ID --metrics --verbose` when you need evidence about unknown unknowns inside the claimed boundary.
4. Treat `special modules` as the centralized view, not the central source of truth. Prefer live module declarations near the owning code and use the CLI to assemble the whole picture.
5. Treat `@area` as structure only. If the real question is shipped behavior or contract honesty, switch to `special specs`.
6. Run `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special lint` if ownership looks malformed or the attachments seem inconsistent.

Read [references/validation-checklist.md](references/validation-checklist.md) for the review rubric and [references/trigger-evals.md](references/trigger-evals.md) for trigger examples.
