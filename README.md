# VisionCache

Windows Computer Use에서 모델에 전달하는 스크린샷을 줄이는 로컬 도구야. 기본 모드는 창 전체를 절반 크기로 보내고, 직전 관찰 이후 작은 글자나 붉은 경고 문구가 나타난 위치만 원본 해상도로 추가해.

## 설치와 실행

Windows에서 MCP로 사용하려면 Windows 10/11의 대화형 데스크톱과 Python 3.10 이상이 필요해. 이미지 비교 함수는 Pillow가 동작하는 다른 운영체제에서도 사용할 수 있어.

```powershell
python -m venv .venv
& .\.venv\Scripts\python.exe -m pip install -e ".[mcp]"
& .\.venv\Scripts\visioncache-mcp.exe --help
```

MCP 클라이언트에서 VisionCache를 **stdio 서버**로 등록하고 실행 파일은 설치한 가상환경의 절대 경로를 사용해. `mcpServers`를 받는 클라이언트의 설정 예시는 다음과 같아.

```json
{
  "mcpServers": {
    "visioncache": {
      "command": "C:\\path\\to\\VisionCache\\.venv\\Scripts\\visioncache-mcp.exe",
      "args": ["--scale", "0.5"]
    }
  }
}
```

MCP 서버는 Windows 창을 캡처하고 도구 결과로 내보내는 역할을 해. API 키는 필요 없어. 도구 결과를 모델 공급자에게 보낼지는 연결한 클라이언트가 결정해.

## MCP 도구

| 도구 | 용도 |
| --- | --- |
| `list_windows` | 제목이 있고 최소화되지 않은 창과 핸들 조회 |
| `observe_window` | `window` 핸들로 관찰. JSON 메타데이터에 이어 PNG 이미지를 반환 |
| `map_point` | 이미지 안의 점을 Windows 화면 픽셀 좌표로 변환. 기존 입력 실행기와 연동할 때 사용 |
| `click`, `scroll` | 가장 최근 관찰의 `observation_id`와 이미지 번호, 이미지 내 좌표로 전경 창 조작 |
| `fill` | 입력칸 클릭과 텍스트 입력을 한 번에 실행. 기존 값을 바꿀 때는 `replace=true` |
| `type_text`, `press_key` | 가장 최근 관찰의 `observation_id`로 전경 창에 유니코드 텍스트 또는 키보드 단축키 입력 |

`observe_window`의 첫 PNG(`image=0`)는 창의 클라이언트 영역 전체야. 다음 PNG는 원본에서 잘라낸 영역이며, 메타데이터의 `source_box`는 원본 이미지에서의 범위를 알려줘. 좌표는 선택한 이미지의 왼쪽 위 기준으로 넘기면 돼. 다른 관찰을 만들거나 입력을 한 뒤에는 새 `observation_id`를 받아야 해.

창이 이동하거나 크기가 달라지면 다시 관찰해. 입력은 대상 창이 전경에 있고 좌표가 가려지지 않았을 때 처리돼. 글씨가 읽기 어렵거나 상위 화면 정보가 필요하면 `mode="full"`로 다시 관찰할 수 있어. 일부 하드웨어 가속 창, 별도 팝업, 관리자 권한 창은 Windows의 캡처와 입력 제한을 받을 수 있어. 이미 내장된 다른 Computer Use 도구에도 적용하려면 에이전트가 VisionCache의 관찰 도구를 호출하도록 연결해야 해.

본창 핸들을 계속 요청하면 `observe_window`가 같은 프로세스에서 그 창이 소유한 전경 대화상자를 따라가고, 닫히면 본창으로 돌아와. 응답의 `requested_window`는 최초 대상, `window`는 실제 잡힌 창이야. 조건부 차분은 대화상자 안의 작은 텍스트를 보존하도록 해당 창을 원본으로 보내고 `effective_mode`에 `full`을 기록해. `follow_owned=false`를 지정하면 요청한 창만 캡처해.

### Python에서 사용

```python
from visioncache import observe

# current와 previous는 캡처에서 받은 Pillow 이미지. 첫 관찰의 previous는 None.
frames = observe("adaptive_delta", current, previous, scale=0.5)
for frame in frames:
    print(frame.image.size, frame.box, frame.reason)
```

`full`은 원본, `resized`는 축소본, `resized_delta`는 축소본과 일반 변경 영역, `adaptive_delta`는 축소본과 조건부 변경 영역이야. 조건부 모드는 글자의 픽셀 높이와 붉은색 변화를 사용해 추가 캡처 영역을 선택해. 다른 앱에 도입할 때는 글꼴·색상·DPI 조건에서 재현해보는 게 좋아.

## 벤치마크

재고 입력과 오류 복구를 각각 네 번씩, 각 방식에서 총 8번 시도한 `gpt-6.2-sol` 실측이야. 비용은 API 사용량에 당시의 표준 요율을 적용했고, 중앙값은 성공한 실행의 완료 시간이야.

| 방식 | 성공 | 성공 1건당 USD | 완료 중앙값 |
| --- | ---: | ---: | ---: |
| 원본 전체 | 8/8 | $0.017472 | 12.41초 |
| 축소 전체 | 8/8 | $0.010330 | 13.01초 |
| 축소 + 일반 차분 | 8/8 | $0.009810 | 11.69초 |
| 축소 + 조건부 차분 | 8/8 | $0.009526 | 11.85초 |

조건부 차분은 56회 중 12회에만 원본 영역을 붙였어. 일반 차분은 56회 중 48회에 원본을 붙였어. 결과는 이 테스트 창과 해당 모델에 한정돼.

[벤치마크 재현 방법](benchmarks/README.md) · [2026-11-22 실측 CSV](benchmarks/reference-2026-11-22.csv)

## 검증

```powershell
& .\.venv\Scripts\python.exe -m pip install -e ".[test]"
& .\.venv\Scripts\python.exe -m pytest -q
& .\.venv\Scripts\python.exe -m tests.windows_mcp_smoke
& .\.venv\Scripts\python.exe -m benchmarks.run --smoke
```

Windows MCP 검증은 임시 재고 입력 창을 열고 실제 마우스·키보드 입력을 보내. 테스트 중에는 그 창을 전경에 둬.
