/**
@spec PROJECTOR.DISTRIBUTION.RELEASE_FLOW.CHECKLIST
Before publishing, projector's local release script interactively confirms easy-to-forget release tasks such as updating public docs, updating `CHANGELOG.md`, bumping workspace package versions, and running core validation.

@spec PROJECTOR.DISTRIBUTION.RELEASE_FLOW.DRY_RUN
Projector's local release script dry-run prints the prerelease checklist and publication commands without moving the `main` bookmark, creating a tag, pushing to origin, or updating Homebrew.

@spec PROJECTOR.DISTRIBUTION.RELEASE_FLOW.MATCHES_WORKSPACE_VERSION
Projector's local release script requires the requested release tag to exactly match the shared workspace package version across projector crates.

@spec PROJECTOR.DISTRIBUTION.RELEASE_FLOW.PUSHES_MAIN_AND_TAG
Projector's local release script publishes the chosen revision by pushing the `main` bookmark with Jujutsu and the release Git tag to origin.

@spec PROJECTOR.DISTRIBUTION.RELEASE_FLOW.VERIFIES_GITHUB_RELEASE
After pushing the release tag, projector's local release script waits for the GitHub Release artifacts to publish and verifies the expected asset set.

@spec PROJECTOR.DISTRIBUTION.RELEASE_FLOW.UPDATES_HOMEBREW
After the GitHub Release is published, projector's local release script updates the Homebrew tap formula for `projector` and verifies that formula against the published projector CLI release archives.

@spec PROJECTOR.DISTRIBUTION.GITHUB_RELEASES.WORKFLOW
Projector keeps a GitHub Actions release workflow in `.github/workflows/release.yml`.

@spec PROJECTOR.DISTRIBUTION.GITHUB_RELEASES.ASSET_MANIFEST
Projector keeps an in-repo release asset manifest that names the GitHub release assets and Homebrew-relevant projector CLI archives expected by its release verification tooling.

@spec PROJECTOR.DISTRIBUTION.HOMEBREW.FORMULA_AUTOMATION
Projector ships Homebrew tap update and verification automation for `Formula/projector.rb` in `LabLeaks/homebrew-tap`, scoped to the user-facing `projector` CLI binary rather than the remotely deployed projector-server binary.
*/
