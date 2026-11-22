# VisionCache Windows 벤치마크

Windows의 테스트용 재고 입력 앱에서 동일한 작업을 네 가지 화면 전달 방식으로 비교한다. 외부 업무 앱을 열지 않으며 모델 동작은 이 테스트 창으로 제한된다.

| 모드 | 모델에게 주는 이미지 |
| --- | --- |
| `full` | 테스트 창 원본 `1024×640` 전체 |
| `resized` | 테스트 창 전체를 축소한 이미지. 기본 `512×320` |
| `resized_delta` | 동일한 축소 전체 화면과 직전 캡처 대비 변경 영역의 원본 크롭. 첫 프레임, 변화 없음, 크롭 면적이 원본의 25%를 넘는 경우에는 축소 화면만 보냄 |
| `adaptive_delta` | 동일한 축소 전체 화면에 작은 글씨나 붉은 경고 문구로 감지된 변경 영역만 원본으로 추가 |

`adaptive_delta`는 화면 픽셀로 판단한다. 붉은 텍스트가 새로 생겼거나, 변경된 글자의 획 높이가 축소본에서 6픽셀 이하로 추정되면 주변 여백을 포함해 원본을 자른다. 첫 프레임, 감지할 단서가 없는 변화, 크롭 면적이 원본의 25%를 넘는 경우에는 축소 화면만 보낸다. 오류의 뜻을 읽는 방식은 아니므로 다른 앱의 테마와 글꼴에서는 감지 성능을 다시 검증해야 한다.

## Windows PowerShell에서 실행

Python 3.10 이상과 Tkinter가 있는 대화형 Windows 데스크톱이 필요하다. 테스트 창이 열리면 조작하지 않은 상태로 둔다. 중단하려면 창을 닫거나 콘솔에서 Ctrl+C를 누른다.

```powershell
python -m pip install -e .
python -m benchmarks.run --smoke
```

`--smoke`는 실제 Windows 창과 캡처 경로를 사용하고 사전에 정한 좌표 동작으로 검증한다. API 호출이 없어 비용 칸은 비어 있다.

유효한 OpenAI API 키가 현재 PowerShell의 `OPENAI_API_KEY`에 설정돼 있으면 아래 명령으로 실측한다. API에 전송하는 화면과 입력값은 재고 입력 테스트에서 생성한 가상 데이터다.

```powershell
# 현재 세션에 키가 설정돼 있지 않을 때만 실행
$env:OPENAI_API_KEY = [System.Net.NetworkCredential]::new('', (Read-Host 'OpenAI API key' -AsSecureString)).Password

# 빠른 실측. 시나리오 2개 × 방식 4개 × 1회
python -m benchmarks.run --repeats 1

# 기본 실측. 시나리오 2개 × 방식 4개 × 3회
python -m benchmarks.run

# 모드마다 실행 순서를 한 번씩 균형 있게 배치
python -m benchmarks.run --repeats 4

# 오류 복구만 반복하고 결과를 다른 경로에 저장
python -m benchmarks.run --scenario recover-bin --repeats 6 --output .\benchmark-results
```

기본 모델은 `gpt-6.2-sol`이다. 네 방식 모두 같은 모델과 요청 설정을 쓰고, 관찰마다 현재 이미지와 직전 동작만 준다. `--model`로 모델을, `--scale`로 축소 비율을 바꿀 수 있다. `--rates INPUT CACHED WRITE OUTPUT`에는 선택한 모델의 100만 토큰당 USD 단가를 기존 단가 대신 넣을 수 있다. 모델은 Responses API 이미지 입력, Structured Outputs, low reasoning, 30분 프롬프트 캐시 설정을 지원해야 한다.

## 실행 결과 읽기

실행마다 `benchmarks/results/live-…/` 또는 `benchmarks/results/smoke-…/`에 결과를 저장한다.

| 파일 | 내용 |
| --- | --- |
| `summary.csv` | 전체 및 시나리오별 성공률, 사용량 기준 총 USD, 성공 1건당 USD, 성공 실행의 중앙 완료 시간 |
| `trials.csv` | 개별 시도의 결과 및 소요 시간, 토큰, API 호출 횟수, 비용 |
| `turns.jsonl` | 단계별 이미지 크기와 영역, 크롭 선택 이유, 동작 및 변환 좌표, API 응답 ID, 실제 토큰 사용량 및 API 왕복 시간 |
| `run.json` | 모델, 단가, 시드, 반복 수, 예정 및 실행 건수, 중단 여부 |

- **성공 판정**은 모델의 완료 응답과 별개로, 앱에 SKU·수량·보관함·확인 코드가 모두 맞게 저장됐는지 확인한다. 두 시나리오 모두 확인 코드를 화면에서 읽어야 한다. `recover-bin`은 추가로 앱이 제시하는 대체 보관함을 사용한다. 한 반복 안에선 네 방식에 같은 값을 주고, 반복마다 방식의 실행 순서를 회전한다.
- **비용**은 Responses API가 돌려준 실제 토큰 사용량에 Standard 요율을 적용한 USD 값이다. 일반 입력, 캐시 적중, 캐시 생성 및 출력(추론 포함)을 각각 계산한다. 성공 1건당 비용은 실패 시도 비용도 포함한 합계를 성공 건수로 나눈다. 사용량이 누락된 요청이 있으면 해당 합계는 비워 두며, `known_api_cost_usd`에는 확인된 부분만 기록한다. 계정별 할인·크레딧은 결제 내역에서 확인한다. [OpenAI 요금표](https://developers.openai.com/api/docs/pricing) · [Prompt caching 비용](https://developers.openai.com/api/docs/guides/prompt-caching)
- **완료 시간**은 초기 상태가 준비된 뒤 첫 화면 캡처부터 마지막 동작 및 앱 상태 판정까지다. 이미지 처리, API 요청, GUI 조작을 포함한다. `median_completion_seconds`는 성공 건에 한해 집계한다. 성공과 실패를 모두 포함한 시간은 `median_attempt_seconds`에 있다.
- **해석 범위**는 이 창과 두 작업에 한정된다. 확인 코드와 오류 안내는 의도적으로 작은 글씨로 배치했다. 다른 앱이나 글꼴 배율에서 성능을 일반화하려면 해당 앱으로 다시 측정해야 한다.

## 한글 2020 실앱 비교

960×1040 클라이언트 영역의 한글 2020에서 새 빈 문서에 기본 2×2 표를 삽입하고 창을 전경에 둔다. 미리보기 배율은 쪽 맞춤으로 둔다. VisionCache MCP의 `list_windows`로 이 테스트 문서의 핸들을 조회한다.

```powershell
$windowHandle = [int](Read-Host '테스트 문서 창 핸들')
python -m benchmarks.run_hwp --window $windowHandle --repeats 3 --no-cache
```

실행기는 매 시도를 세로 설정으로 초기화한다. 편집 용지 대화상자를 연 뒤 모델이 가로 선택, 설정 적용, 인쇄 미리보기 열기, 페이지 수 및 표의 전체 테두리 판독을 수행한다. 모델의 클릭은 이 창의 정해진 세 컨트롤 영역에 들어왔을 때만 실행된다. DPI나 창 모양이 달라질 경우에 맞춰 좌표 영역을 별도로 검증해야 한다. 미리보기에서 끝나는 시나리오다. 화면 전환 직후에는 두 번 연속 같은 화면을 확인하고 모델을 호출한다.

두 모드 모두 본창 핸들로 시작해 연결된 전경 대화상자를 따라간다. `adaptive_delta`는 작은 대화상자를 원본으로 보내므로 이 비교에서 주로 미리보기와 편집 화면의 전송 비용이 달라진다. 측정 시간에 모델 호출 및 UI 처리 시간을 포함하며 시도 간 세로 설정 복원 시간은 제외한다. `--no-cache`를 빼면 API의 기본 암시적 프롬프트 캐시가 적용된다.

[한글 2020 실측 CSV](reference-hwp-2026-11-22.csv)에 화면 안정화 전 예비 결과와 안정화 후 결과가 있다. 안정화 후 캐시를 끈 비교에서 두 모드 모두 3/3 성공했다. 비용은 원본 `$0.009586` 대비 조건부 방식 `$0.006054`로 성공 1건당 36.8% 감소했다.
