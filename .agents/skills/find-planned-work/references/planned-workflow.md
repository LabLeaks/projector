# Planned Workflow

Use this workflow when you need the not-yet-live roadmap:

1. Run `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs --all`.
2. Focus on `[planned]` nodes, not live ones.
3. Scope to a subtree with `MISE_CACHE_DIR=/tmp/projector-mise-cache mise exec -- special specs --all SPEC.ID` if the tree is noisy.
4. Keep planned claims local and exact; do not infer broader commitments than the claim text says.
