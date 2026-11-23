# VisionCache

**AI 에이전트의 데스크톱 컴퓨터 유즈에 드는 입력 토큰을 줄인다.**

VisionCache는 Windows와 macOS에서 AI 에이전트가 MCP로 데스크톱 앱을 보고 조작할 수 있게 한다. 선택한 창의 축소 화면을 보내고, 작은 글씨나 붉은 경고가 나타나면 필요한 영역을 원본 해상도로 보낸다. 클릭, 스크롤, 키 입력과 전경 대화상자 추적을 지원한다.

Windows 문서 편집기 실측에서 **API 입력 토큰이 42.2% 줄었다.** 실행당 **4,188개에서 2,422개**로 줄었고, 두 방식 모두 3회 중 3회 성공했다. [실측 결과](#api-사용량-실측)

[영문 README](README.md) · [npm](https://www.npmjs.com/package/visioncache) · [설치 가이드](INSTALL.md) · [실험 재현](benchmarks/README.md)

## 빠른 시작

에이전트가 조작할 PC에서 설치한다. npm 패키지는 **Windows x64와 Node.js 18 이상**을 지원한다. macOS 14 이상에서는 [소스에서 빌드](#macos)한다.

### 에이전트에게 설치 맡기기

Codex, Claude Code 또는 Gemini CLI의 로컬 세션에 이 문장을 붙여넣는다.

> https://raw.githubusercontent.com/HosooJeong/VisionCache/main/INSTALL.md 를 읽고, 이 PC에서 쓰는 에이전트에 VisionCache 설치부터 MCP 서버와 데스크톱 스킬 등록, 연결 확인까지 직접 해줘.

### npm으로 Windows에 설치

PowerShell에서 패키지를 설치하고 Codex에 등록한다.

```powershell
npm.cmd install --global visioncache
visioncache.cmd --install-codex
```

새 로컬 Codex 채팅을 열면 MCP와 스킬을 불러온다. Claude Code와 Gemini CLI는 [클라이언트별 설치 가이드](INSTALL.md)를 참고한다.

텍스트 편집기를 열어둔 상태에서 다음처럼 요청해 볼 수 있다.

> VisionCache로 열려 있는 텍스트 편집기 창을 확인하고 화면에 보이는 컨트롤을 알려줘.

Codex에서 도구를 지정하고 싶으면 `$visioncache-desktop` 스킬을 호출한다.

## API 사용량 실측

Windows의 한글 2020에서 문서를 가로 방향으로 바꾸고 인쇄 미리보기의 페이지 수와 표 테두리를 확인했다. Python 참조 구현으로 OpenAI API를 호출했고, 각 방식의 캐시 적중과 쓰기는 0이었다.

| 캡처 방식 | 성공 | 실행당 입력 토큰 | 성공 1건당 API 비용 | 완료 시간 중앙값 |
| --- | ---: | ---: | ---: | ---: |
| 원본 해상도 | 3/3 | 4,188 | $0.009586 | 7.56초 |
| 조건부 캡처 | 3/3 | 2,422 | $0.006054 | 7.44초 |

입력과 출력을 합친 토큰은 **41.0% 감소**했고, API 사용량에 당시 실험 단가를 적용한 비용은 **36.8% 감소**했다. 별도의 Windows 재고 입력 실험에서는 네 방식 모두 8회 중 8회 성공했고 조건부 캡처의 성공당 비용이 원본 해상도 대비 **45.5% 줄었다.** [실측 CSV](benchmarks/reference-hwp-2026-11-22.csv) · [영문 비교표와 실험 방법](README.md#measured-api-usage)

## MCP 도구 사용

1. `list_windows`에서 조작할 창을 찾고, 핸들을 `observe_window`에 넘긴다. 이미지 0이 창 전체이고, 뒤에 오는 이미지는 원본 해상도 크롭이다.
2. 이미지 내 좌표와 최신 `observation_id`를 사용해 `click`, `scroll`, `fill`을 호출한다. `type_text`와 `press_key`는 현재 키보드 포커스를 사용한다.
3. 입력 후 다시 관찰한다. 같은 창 핸들을 넘기면 소유 관계가 확인된 전경 대화상자를 따라간다. 세부 내용을 확인할 때는 `mode="full"`로 원본을 요청할 수 있다.

[동작 흐름](README.md#how-it-works) · [캡처 모드와 MCP 설정 예시](README.md#use-the-mcp-tools)

## 소스 빌드와 검증

Windows의 Rust MSVC 빌드와 Codex 등록 방법은 영문 README에 있어. [Windows 소스 빌드](README.md#build-from-source-on-windows) · [Codex 등록](README.md#register-a-source-build-in-codex) · [테스트 명령](README.md#validation)

### macOS

Apple Silicon과 Intel 타깃은 Rust 교차 검사를 통과했다. 실제 Mac에서 화면 캡처와 입력을 검증하는 작업은 남아 있다. Rust와 Xcode Command Line Tools가 설치된 macOS 14 이상에서 빌드한다.

```sh
MACOSX_DEPLOYMENT_TARGET=14.0 cargo build --locked --release
./target/release/visioncache-mcp --permissions
```

빌드한 실행 파일을 로컬 MCP에 등록한다. 캡처에는 화면 기록 권한이, 입력에는 손쉬운 사용 권한이 필요하다. [macOS 설정과 실기기 검증](docs/macos-design.md)

## 라이선스

이 저장소의 소프트웨어 라이선스는 미지정 상태다. [영문 라이선스 안내](README.md#license-status)
