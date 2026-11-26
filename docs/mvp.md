# Agent Monitor — MVP implementation brief

상태: Draft · 2026-11-26

## 1. Goal

기존 Codex CLI 및 Claude Code 실행 방식을 유지한 채 여러 작업을 한 화면에서 관찰한다. 특정 프로젝트(NF95 등)와 무관하게 사용할 수 있어야 한다. MVP는 로컬 단일 사용자용이며 멀티 호스트, 원격 제어 및 복잡한 업무 자동화를 포함하지 않는다.

## 2. User experience

- 브라우저에서 `http://127.0.0.1:8787` (기본값, 설정 변경 가능)을 열면 전체 세션을 확인한다.
- 상단에 실행 중 / 응답 필요 / 완료 수를 표시한다. 승인 대기와 입력 대기는 응답 필요로 집계하되 카드에서 구분한다.
- 카드에는 에이전트, 프로젝트명(작업 폴더명), 선택적 작업 라벨, 현재 상태, 마지막 상태 변경 시각, 경과 시간을 표시한다.
- 대기 세션을 먼저 정렬하며 에이전트, 프로젝트, 상태 필터를 제공한다.
- 종료된 세션은 기본 목록에서 숨기고 최근 기록으로 확인한다. 색상 외에 텍스트와 아이콘으로 상태를 표시한다.
- 완료/대기 알림은 사용자가 명시적으로 활성화한 경우에만 발송한다. 브라우저 알림의 백그라운드 동작 한계를 명시한다.
- 창 이동은 지원 가능한 환경에서만 노출하며 MVP의 필수 안정성 기능과 분리한다. 확인되지 않은 창을 임의로 활성화하지 않는다.

## 3. Adapter contract

우선 Codex CLI, Claude Code 두 종류를 지원한다. 구현 전에 설치된 각 제품 버전의 공식 Hook 문서와 실제 payload를 조사한다. 이벤트 이름, 식별자, 승인/입력 이벤트, 종료 이벤트를 검증한 뒤 지원표를 기록한다. CLI와 Desktop은 동일하다고 가정하지 않는다. Desktop 앱은 검증 전까지 미지원이다.

정규화 이벤트 예시(실제 공급자 payload가 아니라 내부 계약):

```json
{
  "schemaVersion": 1,
  "eventId": "unique-id",
  "source": "codex",
  "hostId": "local-wsl",
  "sessionId": "provider-session-id",
  "turnId": "optional-provider-turn-id",
  "type": "turn.started",
  "occurredAt": "2026-11-26T05:00:00.000Z",
  "cwd": "/home/user/project",
  "label": "optional-manual-label"
}
```

- 식별 키는 source + hostId + sessionId. turnId는 실제로 제공될 때 사용하며 없는 경우 불확실한 상관관계를 추측하지 않는다.
- session.started, turn.started, tool.started, tool.finished, approval.requested, approval.resolved, input.requested, input.received, turn.completed, turn.failed, turn.interrupted, session.ended 등을 내부 타입 후보로 정의한다. 공급자가 제공하지 않는 이벤트는 생성하지 않는다.
- 원문 프롬프트, 모델 응답, 실행 명령, 도구 인자, 환경변수, 인증정보는 수집·저장하지 않는다. 필요하면 수동 라벨을 사용한다.
- Hook 수집기는 기존 Hook을 덮어쓰지 않고 추가하는 방식으로 설치하며, 백업/제거 방법을 제공한다. 연결 실패 시 짧은 제한 시간 내 종료하며 권한 정책에 영향을 주지 않는다.
- 설치·제거는 멱등성을 갖춘다. Windows, WSL2에 대해 각각 검증한다. 브리지 필요 시 최소 범위의 연결만 허용하고 인증 없이 LAN에 노출하지 않는다.

## 4. State reducer

- 요청 시작 → RUNNING; 명시적 승인/입력 요청 → 해당 대기 상태; 해결 → 실행 상태가 확인된 경우 RUNNING, 그렇지 않으면 UNKNOWN.
- 명시적 정상 요청 완료 → COMPLETED; 실패 → ERROR; 중단 → INTERRUPTED; 세션 종료 → ENDED.
- 세션 시작 후 처리 요청 없음 → IDLE. 이미 완료된 요청의 세션은 다음 요청 시작까지 COMPLETED를 유지할 수 있다.
- 동일 eventId 중복 수신은 무시한다. 뒤늦은 이벤트로 새 요청 상태를 되돌리지 않는다. 시각 또는 요청 상관관계로 순서를 확정할 수 없다면 불확실성을 표시한다.
- 수집기 재시작, 이벤트 유실, 장시간 이벤트 부재는 완료나 오류로 단정하지 않는다. 마지막 확인 시각과 관측 불확실성을 표시한다. 신뢰 가능한 heartbeat/liveness가 존재할 때만 적극 활용한다.
- 도구 1개의 실패가 전체 요청 실패임을 뜻하지 않는다. 부모 세션과 서브에이전트를 중복 집계하지 않는다.

## 5. Local API proposal

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/` | dashboard |
| GET | `/api/sessions` | current snapshot |
| GET | `/api/events` | SSE; 초기 snapshot 및 상태 변경 |
| POST | `/api/ingest` | adapter events (authenticated) |
| GET | `/healthz` | collector health |

SSE는 재연결 시 현재 snapshot을 다시 받으며 중복 이벤트로 상태가 오염되지 않아야 한다. 입력 검증, 이벤트 크기 제한, 로컬 수집 토큰, 허용 Host/Origin 검증을 적용한다. 명령 실행·권한 승인 API는 구현하지 않는다. 경로 등 UI 데이터는 HTML로 실행하지 않고 텍스트로 렌더링한다.

## 6. Suggested layout

```text
agent-monitor/
  README.md
  docs/mvp.md
  package.json
  src/
    server.js
    reducer.js
    store.js
    adapters/
      codex.js
      claude-code.js
  public/
    index.html
    app.js
    styles.css
  scripts/
    install-hooks.js
    uninstall-hooks.js
  test/
    reducer.test.js
    server.test.js
    adapters.test.js
    fixtures/
```

파일 구조는 제안이며 필요에 따라 단순화한다. 첫 구현 PR에는 `npm start` 및 `npm test` 명령, 설치/제거 절차, 지원 버전, 제한 사항을 문서화한다.

## 7. Acceptance criteria

- [ ] Windows 및 WSL2에서 수집 서버 실행과 브라우저 접근을 확인한다.
- [ ] Codex CLI와 Claude Code의 실제 테스트 세션에서 공식 이벤트를 수집한다. 지원되는 상태와 미지원 상태를 구분한다.
- [ ] 동시에 네 개 이상의 세션을 실행해 세션 간 상태 오염이 없는지 확인한다.
- [ ] 진행 → 명시적 완료 → 다음 요청 진행, 대기 → 재개, 중단 및 종료를 각각 검증한다. 대기 이벤트가 공급자에서 제공되지 않으면 미지원으로 표시한다.
- [ ] 동일 이벤트 재전송, 순서 역전, 재연결, 모니터 재시작 및 이벤트 유실 시나리오를 검증한다.
- [ ] 수집 서버를 중단한 상태에서도 본래 에이전트 작업과 기존 Hook이 정상 동작한다.
- [ ] UI 갱신에 페이지 새로고침이 필요 없으며 정상적인 로컬 환경에서 이벤트 도착부터 상태 갱신까지 1초 이내를 목표로 한다.
- [ ] 인증 없는 ingest 요청, 잘못된 payload, 과대 payload, 허용되지 않은 Origin/Host를 거부한다.
- [ ] 원문 대화/명령/토큰이 수집 데이터, 스냅샷, 디버그 로그에 남지 않는다.
- [ ] 설치/제거 반복 시 기존 설정 보존 및 추가 Hook 중복 방지를 검증한다.
- [ ] 자동화 테스트 결과 및 실제 OS 검증 결과를 각각 기록한다. 시뮬레이션을 실제 테스트로 보고하지 않는다.

## 8. First implementation request

Implement the smallest useful local monitor, not a general agent orchestration platform. Start by validating official event support with real CLI versions and sanitized fixtures. Implement the normalized reducer and its tests first, then the authenticated local collector, SSE dashboard and hook installation/removal. Keep dependencies minimal. Do not modify external project repositories, grant approvals, launch arbitrary commands or transmit session content. Report unsupported events, cross-OS limitations and actual validation evidence in the first implementation draft PR.
