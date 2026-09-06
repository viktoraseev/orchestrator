---
id: orc-nph1
status: closed
deps: []
links: []
created: 2026-09-06T17:26:42Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести Artifact из format-спеки

Перенести целиком нормативный контракт durable Artifact из format.spec.md в acceptance features: имя и привязку к attempt/Step/InputId, arbitrary bytes, полный набор outputs завершённого attempt, отсутствие output.yaml и read-only/recovery semantics; после исполняемого покрытия удалить раздел и source-теги без удаления существующих сценариев.

## Acceptance Criteria

Все правила Artifact представлены в Rule/Scenario features и наблюдаются через lifecycle API; exact filename, binary bytes, declared InputId, missing/additional artifact и отсутствие output.yaml проверяются явно; раздел Artifact и @format:artifact удалены, ссылки обновлены; существующие сценарии не удалены; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T17:31:10Z**

Перенесён весь контракт Artifact в artifact_completion, recovery и run_inspection: добавлен API-сценарий exact durable filename + regular binary bytes + отсутствие output.yaml; существующие сценарии full snapshot, invalid InputIds, recovery и CLI binary output сохранены. Раздел Artifact и @format:artifact/@format:artifacts удалены, ссылки AGENTS.md и cli.md обновлены. Целевой cargo test --test lifecycle проходит.

**2026-09-06T17:32:24Z**

Финальная проверка: ./scripts/test.sh и ./scripts/build.sh проходят; git diff --check чист. Раздел Artifact и все @format:artifact/@format:artifacts удалены; format.spec.md сохраняет только root/layout и общие YAML rules. Diff-аудит не обнаружил удаления Rule, Scenario или Scenario Outline.
