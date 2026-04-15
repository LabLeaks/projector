# Validation Checklist

Use this rubric when reviewing one live `projector` module:

- Read the exact `@module` text before judging the code that claims to implement it.
- Compare attached `@implements` or `@fileimplements` bodies to the module’s stated responsibility, not to nearby files or names.
- Use `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special modules MODULE.ID --metrics --verbose` when you need architecture evidence beyond direct ownership tracing.
- Prefer direct ownership. A live `@module` without direct `@implements` or `@fileimplements` is architecture drift unless it is explicitly `@planned`.
- Do not use architecture validation to prove product behavior. Switch to `special specs` when the question is what the product ships.
