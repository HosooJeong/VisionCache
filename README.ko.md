# VisionCache

VisionCache는 스크린샷으로 데스크톱 앱을 조작하는 에이전트용 로컬 MCP 서버다. 기본 모드는 선택한 창의 축소 화면을 보내고, 새로 나타난 작은 글씨나 붉은 경고 영역만 원본 크기로 추가한다. 모델이 고른 이미지 좌표를 실제 창의 입력 좌표로 변환한다.

[English](README.md) · [macOS 설계](docs/macos-design.md) · [벤치마크 재현](benchmarks/README.md)

| 구성 | 현재 상태 |
| --- | --- |
| Windows 데스크톱 MCP | Windows 10/11 대상. Windows 10의 테스트 앱과 한글 2020에서 검증했다. |
| Python 이미지 처리 | 다른 캡처 실행기에서 받은 Pillow 이미지를 입력으로 사용할 수 있다. |
| macOS 데스크톱 MCP | macOS 14 이상을 대상으로 설계했다. 네이티브 구현과 실기기 검증은 다음 단계다. |

## 설치

대화형 Windows 데스크톱과 Python 3.10 이상에서 실행한다.

```powershell
git clone https://github.com/HosooJeong/VisionCache.git
cd VisionCache
python -m venv .venv
& .\.venv\Scripts\python.exe -m pip install -e ".[mcp]"
& .\.venv\Scripts\visioncache-mcp.exe --help
```

MCP 클라이언트에 가상환경의 `visioncache-mcp.exe` 절대 경로를 **stdio 서버**로 등록한다. 서버에는 API 키가 필요 없다. 반환된 이미지와 메타데이터의 모델 전송은 클라이언트에서 결정한다. [설정 예시](README.md#quick-start)

## 사용 흐름

1. `list_windows`에서 대상 창을 선택한다.
2. `observe_window`로 관찰한다. 이미지 0은 창 내부 전체이며, 이후 이미지는 변경 영역의 원본 크롭이다.
3. `observation_id`와 해당 이미지 안의 좌표로 `click`, `scroll`, `fill`을 호출한다. 현재 포커스에는 `type_text`와 `press_key`를 사용할 수 있다.
4. 입력 후에는 다시 관찰한다. 입력 대상은 전경 창이어야 하며, 창이 이동하거나 크기가 달라진 경우에는 좌표를 다시 확인한다.

본창 핸들을 계속 사용하면 같은 앱의 소유 관계가 확인된 전경 대화상자를 따라간다. 대화상자가 닫히면 다시 본창을 관찰한다. `requested_window`는 처음 요청한 창, `window`는 실제 캡처한 창이다. `follow_owned=false`를 지정하면 선택한 창을 직접 캡처한다.

| 모드 | 전송 내용 |
| --- | --- |
| `full` | 원본 전체 화면 |
| `resized` | 축소 전체 화면 |
| `resized_delta` | 축소 전체 화면과 일반 변경 영역 |
| `adaptive_delta` | 축소 전체 화면과 작은 글씨·붉은 경고의 변경 영역. 소유 대화상자는 원본 크기로 전송한다. |

감지는 픽셀 크기와 색상에 의존하며 앱 테마와 글꼴 배율에 영향을 받는다. 세부 판독이 필요하면 `mode="full"`로 다시 관찰한다. [도구와 Python 연동](README.md#observe-and-interact)

## 검증 결과

한글 2020에서 가로 방향을 설정하고 인쇄 미리보기에서 1쪽과 표의 테두리를 확인했다. 화면 전환이 안정될 때까지 기다리고 API 프롬프트 캐시를 끈 조건에서 두 방식 모두 3회 전부 성공했다.

| 방식 | 성공 1건당 비용 | 완료 시간 중앙값 |
| --- | ---: | ---: |
| 원본 전체 | $0.009586 | 7.56초 |
| 조건부 차분 | $0.006054 | 7.44초 |

조건부 차분의 API 비용은 이 작업에서 36.8% 낮았다. Windows 재고 입력 테스트에서는 네 가지 모드가 각각 8회 전부 성공했다. [한글 실측](benchmarks/reference-hwp-2026-11-22.csv) · [재고 입력 실측](benchmarks/reference-2026-11-22.csv) · [테스트 실행](README.md#development)

## 라이선스 상태

공개 저장소이며 라이선스 파일은 미지정 상태다. [GitHub 저장소 라이선스 안내](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/licensing-a-repository)
