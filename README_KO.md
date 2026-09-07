# Containbar

*Containers in your menu bar.*

Docker Tray의 새 이름입니다. 기존 설정 호환성을 위해 앱 식별자와 설정 저장 키는 유지합니다.
이전 앱에서 업그레이드하면 Containbar를 실행하고 로그인 시 자동 실행을 다시 설정해 주세요.
기존 Docker Tray.app은 새 앱의 정상 동작을 확인한 뒤 제거할 수 있습니다.

<p align="center">
  <img src="src-tauri/icons/icon.png" width="128" />
</p>

<p align="center">
  macOS 메뉴바에서 Docker·Colima·Apple Container를 관리하는 경량 앱
  <br/>
  Docker Desktop 없이도 동작하는 내장 런타임(Colima) 포함
  <br/>
  <br/>
  <a href="https://www.apple.com/macos/"><img src="https://img.shields.io/badge/macOS-13+-000000?style=for-the-badge&logo=apple&logoColor=white" alt="macOS"></a>
  <a href="https://nodejs.org/"><img src="https://img.shields.io/badge/Node.js-v22.x-339933?style=for-the-badge&logo=node.js&logoColor=white" alt="Node.js"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.77+-DEA584?style=for-the-badge&logo=rust&logoColor=white" alt="Rust"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue?style=for-the-badge" alt="License"></a>
  <br/>
  <br/>
  <a href="./README.md">English</a> / 한국어
</p>

<p align="center">
  <img src="assets/theme-comparison-refraction.png" width="512" alt="Containbar 테마 비교 시안: 왼쪽 Cobalt, 오른쪽 Liquid Glass" />
  <br/>
  <sub>Cobalt / Liquid Glass — AI로 제작한 테마 비교 시안이며 실제 앱 캡처가 아닙니다.</sub>
</p>

## 기능

### Docker 런타임
- **내장 런타임**: Colima 기반 경량 VM을 번들하여 Docker Desktop/OrbStack 없이 독립 실행
- **외부 런타임 호환**: Docker Desktop, OrbStack 등 기존 런타임이 있으면 자동 감지하여 사용
- **Apple Container**: macOS 26 이상 Apple Silicon에서 Apple `container` CLI 지원, Compose 실행 시 Mocker 자동 설치
- **자동 시작**: 앱 실행 시 Docker가 없으면 내장 런타임 자동 시작
- **로그인 시 실행**: Settings에서 "Start at Login" 토글

### 컨테이너 관리
- **시스템 트레이**: 메뉴바에 상주하며 클릭으로 토글, 우클릭으로 Quit
- **컨테이너 관리**: 시작, 중지, 재시작, 삭제
- **그룹 제어**: Compose 그룹 단위 시작/중지/삭제
- **Compose 지원**: `docker-compose.yaml` 파일 불러와서 실행 (Docker/Colima는 Docker Compose, Apple Container는 Mocker 사용). Apple 전환 시 포트를 점유한 기존 Compose 프로젝트를 식별하고, 사용자 확인 후 안전하게 중지하고 계속할 수 있습니다.
- **Apple Compose 자동 복원**: Apple Container에서 `restart: always` 및 `unless-stopped` 정책을 앱이 보존하여 런타임이나 앱 재시작 후 서비스와 네트워크 호스트 매핑을 복원
- **이미지 관리**: Pull, 이미지에서 컨테이너 생성, 삭제
- **볼륨 & 네트워크**: 조회 및 삭제
- **검색/필터**: 모든 탭에서 이름, 이미지, 드라이버로 검색
- **상세 보기**: 아이템 클릭으로 기본 정보 + 환경변수 확인, 우클릭으로 삭제

### 로그 뷰어
- **실시간 로그**: 1초 간격 incremental polling으로 새 로그만 추가
- **Follow tail**: 새 로그 도착 시 자동 스크롤, 수동 스크롤하면 자동 해제
- **타임스탬프 토글**: on/off 전환, 타임스탬프와 로그 본문 시각적 구분
- **텍스트 복사**: 로그 텍스트 드래그 선택 + Cmd+C 복사

### 도구
- **파일 탐색기**: 컨테이너 내부 파일 탐색 및 전송
- **터미널 접속**: 실행 중인 컨테이너에 쉘 접속 (Ghostty, iTerm, Terminal.app 지원)
- **창 크기 조절**: 하단 가장자리 드래그로 높이 조절

## 설치

```bash
curl -fsSL https://raw.githubusercontent.com/yurseria/containbar/main/scripts/install.sh | bash
```

## Docker 런타임

Containbar는 Docker Desktop 없이도 동작합니다.

| | 외부 런타임 | 내장 런타임 |
|---|---|---|
| **방식** | Docker Desktop, OrbStack 등 | 번들된 Colima (경량 VM) |
| **감지** | 앱 시작 시 자동 감지 | 외부 런타임 없을 때 자동 시작 |
| **추가 설치** | 필요 없음 | 필요 없음 (앱에 포함) |
| **앱 크기** | 13MB | ~126MB (Colima + Lima + Docker CLI 포함) |

내장 런타임 첫 실행 시 VM 이미지를 다운로드합니다 (~200MB, 한 번만). 완료되면 macOS 알림으로 안내합니다.

## 기술 스택

- **프론트엔드**: React 19, TypeScript, Vite
- **백엔드**: Rust, Tauri 2, Bollard (Docker API)
- **런타임**: Colima, Lima, Docker CLI (번들)
- **Node**: 22 (`.nvmrc` 참고)

## 사전 요구사항

사용 시:
- macOS 13+

개발 시:
- [Rust](https://rustup.rs/)
- [Node.js 22+](https://nodejs.org/)
- [Colima](https://github.com/abiosoft/colima) (`brew install colima` — 런타임 번들링용)

## 개발

```bash
npm install
npm run dev:tauri
```

## 빌드

```bash
# 런타임 번들링 (첫 빌드 시)
./scripts/bundle-runtime.sh

# 앱 빌드
npm run tauri build
```

빌드된 앱은 `src-tauri/target/release/bundle/`에 생성됩니다.

## 프로젝트 구조

```
├── src/                    # React 프론트엔드
│   ├── components/         # UI 컴포넌트
│   ├── hooks/              # useDocker 훅
│   └── types.ts            # TypeScript 타입
├── src-tauri/              # Rust 백엔드
│   ├── src/
│   │   ├── docker.rs       # Docker API 커맨드
│   │   ├── runtime.rs      # Colima 런타임 관리
│   │   └── lib.rs          # Tauri 앱 설정, 트레이, 윈도우
│   ├── runtime/            # 번들된 바이너리 (git 제외)
│   └── tauri.conf.json     # Tauri 설정
├── scripts/
│   ├── bundle-runtime.sh   # Colima/Lima/Docker CLI 번들링
│   └── install.sh          # 원클릭 설치 스크립트
└── vite.config.ts
```

## 라이선스

MIT
