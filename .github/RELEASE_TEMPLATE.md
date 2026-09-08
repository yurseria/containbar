## Install

### macOS (Homebrew)

Requires Homebrew, macOS 13+, and Apple Silicon.

```bash
brew install --cask yurseria/tap/containbar
```

To update:

```bash
brew update
brew upgrade --cask yurseria/tap/containbar
```

The Tap refresh is triggered after release assets are uploaded. If the new version is not visible yet, wait for the [tap workflow](https://github.com/yurseria/homebrew-tap/actions) to finish and run `brew update` again.

### macOS (manual installation / first launch)

Download the DMG and drag **Containbar** to Applications. The app is not Developer ID signed or notarized. If macOS blocks launch, use **System Settings → Privacy & Security → Open Anyway** only if you trust the app. Homebrew does not bypass Gatekeeper. See [Apple's instructions](https://support.apple.com/en-us/102445).
