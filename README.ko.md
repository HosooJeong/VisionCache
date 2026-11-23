# VisionCache

**AI 에이전트의 데스크톱 컴퓨터 유즈에 드는 입력 토큰을 줄인다.**

Windows 문서 편집기 실측에서 **API 입력 토큰이 42.2% 줄었다.** 실행당 4,188개에서 2,422개로 줄었고, 두 방식 모두 3회 중 3회 성공했다. [실측 결과](#api-사용량-실측)

VisionCache는 AI 에이전트가 MCP로 Windows와 macOS 데스크톱 앱의 화면을 보고 조작할 수 있게 한다. 선택한 창의 축소 화면을 보내고 작은 글씨나 경고가 나타나면 해당 영역을 원본 해상도로 보낸다. 클릭, 스크롤, 텍스트 입력과 전경 대화상자 추적을 지원한다.

[영문 메인 README](README.md) · [실측과 재현](benchmarks/README.md) · [macOS 설계](docs/macos-design.md)

## npm으로 Windows에 설치

Windows x64에서 Node.js 18 이상을 설치한 뒤 PowerShell에서 실행한다. npm 패키지에 VisionCache 실행 파일이 들어 있다. [npm 패키지](https://www.npmjs.com/package/visioncache)

```powershell
npm.cmd install --global visioncache
```

Codex에 MCP 서버와 데스크톱 스킬을 등록하려면 아래 명령을 실행하고 새 로컬 Codex 채팅을 연다.

```powershell
visioncache.cmd --install-codex
```

Claude Code나 Gemini CLI를 쓰면 아래 문장으로 설치를 맡기거나 [설치 가이드](INSTALL.md)를 따르면 된다. macOS에서는 [소스에서 빌드](#macos)한다.

### 에이전트에게 설치 맡기기

조작할 PC에서 Codex, Claude Code 또는 Gemini CLI의 로컬 세션을 열고, 아래 문장을 붙여넣는다.

> https://raw.githubusercontent.com/HosooJeong/VisionCache/main/INSTALL.md 를 읽고, 이 PC에서 쓰는 에이전트에 VisionCache 설치부터 MCP 서버와 데스크톱 스킬 등록, 연결 확인까지 직접 해줘.

## API 사용량 실측

한글 2020에서 문서를 가로로 바꾸고 인쇄 미리보기의 페이지 수와 표 테두리를 확인했다. 조건부 캡처의 입력 토큰은 실행당 4,188개에서 2,422개로 **42.2% 감소**했고, 입력과 출력을 합친 토큰은 **41.0% 감소**했다. 두 방식 모두 3회 중 3회 성공했고, 캐시 적중과 쓰기는 0이었다. API 사용량에 당시 실험 단가를 적용한 성공 1건당 비용은 36.8% 줄었다. [영문 비교표](README.md#measured-api-usage) · [한글 2020 실측 CSV](benchmarks/reference-hwp-2026-11-22.csv)

## Windows에서 소스 빌드

대화형 Windows 데스크톱에서 Rust MSVC 툴체인으로 빌드한다.

```powershell
cargo build --locked --release
.\target\release\visioncache-mcp.exe --help
```

MCP 클라이언트에서 `visioncache-mcp.exe`의 절대 경로를 실행 명령으로 등록한다. 실행 파일 자체에는 Python 런타임이나 모델 API 키가 필요 없다. [MCP 설정 예시](README.md#build-from-source-on-windows)

1. `list_windows`로 창 핸들을 조회한다.
2. `observe_window`에 핸들을 넣는다. 결과는 JSON 메타데이터와 PNG 순서이며, 이미지 0은 창 전체를 보여준다.
3. `observation_id`로 `click`, `scroll`, `fill`, `type_text`, `press_key`를 호출한다. 좌표는 선택한 이미지 기준이다.
4. 입력이 끝나면 최신 화면을 다시 관찰한다. 동일한 본창 핸들을 사용하면 소유 관계가 확인된 전경 대화상자를 자동으로 따라간다.

`adaptive_delta`는 기본 모드다. 화면이 읽기 어렵다면 `mode="full"`로 원본을 요청할 수 있다. 텍스트 크기와 색상이 감지에 영향을 준다. [모드와 좌표 상세](README.md#use-the-mcp-tools)

## 소스 빌드로 Codex에 등록

설치 명령을 한 번 실행하면 현재 사용자 설정에 VisionCache MCP와 데스크톱 자동 선택 스킬을 함께 등록한다.

```powershell
.\target\release\visioncache-mcp.exe --install-codex
```

macOS에서는 `./target/release/visioncache-mcp --install-codex`를 실행한다. Codex CLI 경로를 직접 지정하려면 `--codex-cli`를 사용한다.

새 로컬 Codex 채팅에서 Windows 또는 macOS 데스크톱 앱 조작을 요청하면 자동 선택 대상이 된다. 직접 선택하려면 `$visioncache-desktop`을 호출한다. 앱 설정의 **Computer use** 스위치는 내장 공급자를 제어하며, VisionCache는 등록된 외부 MCP로 연결된다. [Codex 스킬 안내](https://learn.chatgpt.com/docs/build-skills) · [내장 Computer Use](https://learn.chatgpt.com/docs/computer-use)

## 구현과 검증

Win32가 반환한 BGR 픽셀을 Rust 버퍼로 직접 받고, 연속 관찰에서는 두 버퍼를 교대로 사용한다. 차분·크롭 선택·리사이즈·PNG 인코딩·MCP 모두 Rust 실행 파일 안에서 동작한다.

`cargo test --locked`로 공통 코어를 검증한다. Windows에서 기존 통합 테스트의 `--server`에 Rust 바이너리 경로를 주면 시나리오를 실제 GUI로 확인할 수 있다. Python은 이전 구현과 테스트 도구로 남아 있다. 앞서 발표한 재고 입력과 한글 2020 API 실측은 Python 버전 기준이다. [검증 명령](README.md#validation) · [한글 2020 실측](benchmarks/reference-hwp-2026-11-22.csv)

## macOS

macOS 14 이상을 대상으로 ScreenCaptureKit 창 캡처, Accessibility 대상 확인, Quartz 입력을 구현했다. Apple Silicon과 Intel 타깃의 교차 컴파일 검사를 통과했다. 실기기에서 권한, Retina 좌표, 창과 대화상자 동작을 검증해야 한다.

```sh
MACOSX_DEPLOYMENT_TARGET=14.0 cargo build --locked --release
./target/release/visioncache-mcp --permissions
```

MCP에 빌드 결과의 절대 경로를 등록한다. 화면 기록 권한으로 캡처를, 손쉬운 사용 권한으로 입력을 검증한다. 관찰의 좌표 단위는 macOS에서 전역 화면 포인트다. 공개 저장소의 이용 라이선스는 미지정 상태다. [macOS 설계와 실기기 검증 순서](docs/macos-design.md)
