## Install

### macOS (Quick Install)

```bash
curl -fsSL https://raw.githubusercontent.com/yurseria/containbar/main/scripts/install.sh | bash
```

> `curl`, `jq` 필요

### macOS (수동 설치)

macOS 빌드는 Apple 인증서로 서명되지 않았습니다. 브라우저에서 DMG를 다운로드한 경우 Gatekeeper가 앱을 차단합니다.

**방법 1 — 터미널:**
```bash
xattr -rd com.apple.quarantine /Applications/Containbar.app
```

**방법 2 — Finder:**
1. `.dmg`를 열어 **Containbar**를 Applications로 드래그
2. Applications에서 앱을 **우클릭 → 열기** (최초 1회만)
