---
id: orc-1b6y
status: closed
deps: []
links: []
created: 2026-09-06T17:15:40Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести Agent attempt record из format-спеки

Перенести крупным блоком нормативные правила раздела Agent attempt record из format.spec.md в acceptance features, расширить исполняемое покрытие schema, filename binding, input и ordered events и удалить перенесённый текст и source-теги без удаления существующих сценариев.

## Acceptance Criteria

Все правила раздела представлены в Rule/Scenario features; exact pending/completed records, filename binding, input order, event schema/order и invalid histories проверяются через lifecycle API; раздел удалён из format.spec.md и ссылки обновлены; существующие сценарии не удалены; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T17:22:40Z**

Перенесён контракт Agent attempt record в lifecycle, graph_execution и recovery; добавлены exact-schema assertions и 16 invalid durable cases. При переносе acceptance выявил, что Serde unit-вариант completed принимал лишние поля несмотря на deny_unknown_fields; представление заменено на пустой struct-вариант без изменения валидного YAML. Целевой cargo test --test lifecycle проходит.

**2026-09-06T17:24:03Z**

Финальная проверка: ./scripts/test.sh и ./scripts/build.sh проходят. Раздел Agent attempt record и все @format:agent-attempt-record удалены после переноса; diff-аудит не обнаружил удаления существующих Rule/Scenario/Scenario Outline.
