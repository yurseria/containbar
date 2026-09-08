# Homebrew release integration

After the release workflow finishes uploading all artifacts, it calls `update-homebrew.yml`. That workflow dispatches `update-casks.yml` on `yurseria/homebrew-tap` (main). The tap downloads the latest stable DMGs, validates their SHA-256 checksums, and updates the Casks. No new app release is created by the dispatch workflow.

The request is immediate; the Cask becomes available after the tap workflow finishes, not synchronously with the upload. Users then run `brew update` and `brew upgrade --cask` with the app's fully qualified Cask name.

## Authentication

The dispatcher prefers the repository secret `HOMEBREW_TAP_TOKEN`, falling back to the existing release secret `GH_TOKEN`. A fine-grained token needs access to **yurseria/homebrew-tap** with **Actions: read and write**. The app repository's automatic GITHUB_TOKEN cannot dispatch workflows in another repository. Never commit tokens.

## Recovery

If dispatch fails, the workflow reports an error without rolling back the published release. Fix the token permissions and manually run **Update Homebrew tap** in the app repository's Actions tab. This only requests a tap refresh; it does not rebuild or release the app. Confirm completion in the tap's Actions tab. The tap's six-hour scheduled refresh remains a fallback.

Changes to the release workflow must also be included in the next promotion to the `release` branch. Pushing main alone does not publish an app release.
