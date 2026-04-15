# State Walkthrough

Use this workflow when you need the current live state:

1. Run `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs`.
2. Identify the exact claim or subtree you care about.
3. Use `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs SPEC.ID` for a focused tree view.
4. Use `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs SPEC.ID --verbose` when you need the actual support body.
5. Do not infer planned work from this view; it shows live claims only.
