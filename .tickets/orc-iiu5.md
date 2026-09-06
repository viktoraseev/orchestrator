---
id: orc-iiu5
status: closed
deps: []
links: []
created: 2026-09-06T14:47:11Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести сущности из SPEC.md в acceptance features

Разложить весь раздел «Сущности» по тематическим feature Rules, дополнить отсутствующее acceptance-покрытие и удалить раздел вместе с @spec:сущности.

## Acceptance Criteria

Все наблюдаемые контракты Run, Agent type, Agent, Attempt, Process executor, Agent session activation/view, Artifact и Run lock закреплены сценариями; существующие сценарии не удалены; @spec:сущности отсутствует; раздел удалён из SPEC.md; ./scripts/test.sh и ./scripts/build.sh проходят.

## Notes

**2026-09-06T14:57:19Z**

Раздел «Сущности» перенесён: Run — lifecycle/run_inspection; Agent type и Agent — agent_types/human_execution с новым Scenario Outline о неизменности materialized Agent после изменения config; Attempt — graph_execution/lifecycle/recovery/process_execution; Process executor — process_execution; session activation/view — lifecycle/agent_types/control_endpoint с явной проверкой одного run; Artifact — artifact_completion; Run lock — run_lock с новым сценарием unlocked lock-файла. Существующие сценарии не удалялись. Устаревшие ссылки на SPEC.md заменены ссылками на feature Rules. cargo test --test lifecycle, ./scripts/test.sh и ./scripts/build.sh прошли.
