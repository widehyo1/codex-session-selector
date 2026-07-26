# Implementation Plan Authoring Guidelines

## 목적

이 지침은 기능 구현 전에 작성하는 계획 문서를 구현자가 추가 설계 판단 없이
실행할 수 있는 수준으로 만드는 기준이다. 향후 2번 TUI toggle과 3번 검색
강화 계획에도 동일하게 적용한다.

## 필수 원칙

1. 현재 코드와 테스트를 먼저 확인하고, 추측이 아닌 실제 동작을 baseline으로
   기록한다.
2. 구현 대상, 의도적 변경, 유지할 동작, 비범위와 defer 항목을 분리한다.
3. 선택지가 여러 개면 계획 작성 시 하나를 결정한다. `TODO`, `TBD`,
   “구현 시 결정”을 남기지 않는다.
4. 결정할 근거가 없거나 사용자 선택이 필요한 항목만 blocker로 명시한다.
5. 계획 작성 중에는 요청받지 않은 기능 코드를 구현하지 않는다.
6. defer된 기능을 위한 선행 schema 변경이나 숨은 동작을 넣지 않는다.
7. 구현 단위별 quality gate와 커밋 가능 조건을 계획에 포함한다.

## 문서 필수 구성

계획 문서는 다음 순서를 기본으로 한다.

1. 문서 상태와 목표 버전
2. 목표와 사용자에게 보이는 최종 동작
3. 구현 범위와 명시적 비범위
4. 현재 동작 및 호환성 계약
5. 확정한 기술·구조 결정
6. 목표 파일 구조와 파일별 책임
7. 타입과 함수 시그니처
8. 주요 흐름 및 알고리즘 pseudocode
9. 오류 처리, 상태 소유권, lifecycle
10. 단계별 구현 순서
11. red/green 및 커밋 정책
12. pre-commit/CI 자동화
13. unit/integration/manual test
14. package 및 문서 변경
15. 완료 조건
16. 후속 작업 경계

## Pseudocode 작성 규칙

- 대상 언어와 유사한 문법을 사용한다. Rust 구현이면 Rust 형태로 작성한다.
- 함수명, parameter type, return type, ownership 형태를 명시한다.
- struct와 enum은 실제 필요한 field와 variant를 모두 적는다.
- 입력 기본값, 출력, 오류 조건과 side effect를 명시한다.
- 여러 줄 설명은 코드 블록 위에 작성한다.
- 한 줄로 충분한 설명은 해당 코드 바로 위 주석으로 작성한다.
- 프로세스, DB, 파일, terminal 상태가 바뀌는 지점을 숨기지 않는다.
- 외부 명령은 program, argument 순서, exit-status 처리까지 적는다.
- schema를 사용하면 정확한 DDL, key, index, row identity와 transaction
  경계를 적는다.
- 알고리즘은 순서, filter 조건, 정렬, 상태 전이를 구현 가능한 형태로 적는다.

다음과 같은 모호한 표현은 사용하지 않는다.

```text
적절히 처리한다
필요하면 캐시한다
상황에 따라 선택한다
나중에 schema를 정한다
관련 test를 추가한다
```

대신 다음처럼 구체화한다.

```rust
fn search(query: &str, scope: SearchScope) -> Result<Vec<SearchHit>>;

// 모든 공백 분리 term이 같은 row의 선택 scope에 포함되어야 한다.
let matches = terms.iter().all(|term| haystack.contains(term));
```

## 기술 선택 기록

새 기능이나 저장소 기능을 사용할 때 다음을 명시한다.

- 사용할 기술과 정확한 feature
- 필요한 최소 runtime/library version
- 현재 dependency에서 지원되는지 확인 방법
- 선택 이유와 배제한 대안
- 데이터 호환성과 migration 방법
- 실패 또는 미지원 환경의 동작
- 성능 검증 방법

예를 들어 FTS5 Contentless-Delete를 선택한다면 단순히 “FTS5 사용”이라고
쓰지 않는다. 최소 SQLite version, `content=''`,
`contentless_delete=1`, stable `rowid`, 원본 table과의 연결, UPDATE/DELETE
규칙, transaction 및 rebuild 전략을 명시한다. 공식 SQLite 문서와 현재
bundled SQLite version을 구현 전에 확인한다.

## 테스트 작성 기준

“테스트 추가”라는 문장만 두지 않고 다음을 포함한다.

- test 함수명
- fixture 입력
- 호출할 함수 또는 CLI
- 기대 상태와 exact assertion
- 오류 case
- 기능이 추가되지 않았음을 확인하는 negative test
- 자동화하기 어려운 TUI 기능의 manual smoke 절차

각 구현 단계는 독립적으로 test가 통과해야 한다. 최종 계획에는 최소한 다음
검증을 포함한다.

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
git diff --check
```

package나 binary 구성이 바뀌면 임시 설치 경로에서 실제 산출물도 확인한다.

## Red/green 및 커밋 정책

기본 정책은 green-only commit이다.

각 구현 단위는 다음 순서를 따른다.

```text
새 test 또는 회귀 test 작성
  ↓
대상 test만 실행하여 예상한 이유로 실패(red)하는지 확인
  ↓
같은 구현 단위에서 production code 작성
  ↓
대상 test 통과(green)
  ↓
전체 pre-commit quality gate 통과
  ↓
test와 구현을 함께 커밋
```

- red 상태 자체는 커밋하지 않는다.
- red 실행 결과는 현재 세션의 작업 기록이나 PR 설명에 남길 수 있지만 git
  history를 깨진 상태로 만들지 않는다.
- phase의 의미는 test 이름, 구현 코드와 commit message로 남긴다.
- 기능 구현 없이 test만 추가해도 처음부터 green인 characterization test는
  독립 커밋할 수 있다.
- 긴 작업을 중단해야 해도 실패하는 코드를 임시 커밋하지 않는다. working
  tree 또는 별도 draft branch 사용 여부는 사용자 지시에 따른다.

red commit을 허용해야 하는 특별한 workflow가 있다면 계획 문서에서 이유,
branch 정책, CI 허용 범위를 별도로 승인받는다.

## Pre-commit과 CI 자동화

검사 명령을 hook이나 workflow YAML에 중복 작성하지 않는다. version
control되는 공통 shell script를 single source of truth로 사용한다.

기본 파일:

```text
scripts/check-before-commit.sh
.githooks/pre-commit
scripts/install-git-hooks.sh
.github/workflows/ci.yml
```

공통 검사 순서:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

- `.githooks/pre-commit`은 공통 script만 `exec`한다.
- GitHub Actions도 같은 script만 호출한다.
- hook installer는 기존 `core.hooksPath`를 조용히 덮어쓰지 않는다.
- baseline clippy warning이 있으면 hook 도입 전에 동작 변화 없는 별도
  bootstrap phase에서 제거한다.
- `--no-verify`로 hook을 우회한 commit도 CI에서 동일 검사를 통과해야 한다.
- script 자체는 `bash -n`으로 syntax를 검사한다.

## 호환성과 defer 처리

- 정상 동작뿐 아니라 기본값, option 전달, 오류 후 복귀, 출력과 상태 유지도
  호환성 계약에 포함한다.
- breaking change는 의도적 변경 목록과 migration mapping에 적는다.
- defer 항목은 비범위 목록, negative test, 완료 조건에 반복해서 고정한다.
- 향후 기능을 쉽게 만들기 위한 module boundary는 허용하지만, 해당 기능의
  상태·schema·key·query를 미리 추가하지 않는다.

## 2번 계획 작성 시 추가 확인 항목

2번 TUI toggle 계획에는 최소한 다음 결정이 필요하다.

- toggle key와 기존 key 충돌 여부
- 상태를 소유하는 struct와 초기값
- command-line `--include-exec`와 toggle의 관계
- selector와 replay 각각에서 toggle이 의미하는 범위
- 화면 이동과 복귀 시 상태 유지 규칙
- 이미 parse한 entry의 filter/rebuild 알고리즘
- empty selection, search mode, help/fullscreen에서 key 처리
- 상태 표시 위치와 exact 문구
- 상태 전이 unit test 및 TUI manual smoke test

## 3번 계획 작성 시 추가 확인 항목

3번 검색 강화 계획에는 최소한 다음 결정이 필요하다.

- 검색 대상 column과 exec 포함 정책
- tokenizer, query 문법, AND/OR, prefix, phrase 및 escaping 규칙
- ranking 방식과 동일 score 정렬
- canonical table과 FTS table의 관계
- stable row identity
- Contentless-Delete DDL과 SQLite 최소 version
- insert/update/delete transaction 순서
- 기존 DB migration 또는 rebuild 절차
- index 불일치 복구 방법
- 한글, 경로, repository URL, command text test fixture
- 기존 부분 문자열 검색과 결과가 달라지는 의도적 사례
- 데이터 크기별 index 시간, DB 크기, query latency benchmark

2번과 3번은 각각 별도 계획 문서로 작성한다. 두 기능 사이에 공통 변경이
필요하면 어느 계획이 선행하고 어떤 API를 제공하는지 명시한다.
