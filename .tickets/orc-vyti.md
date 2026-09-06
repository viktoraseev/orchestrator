---
id: orc-vyti
status: closed
deps: []
links: []
created: 2026-09-06T14:14:50Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести главный workflow из SPEC.md в acceptance features

Перенести весь нормативный раздел «Главный workflow» в тематические Rule и сценарии features без дублирования внутреннего алгоритма, затем удалить раздел и его source-теги.

## Acceptance Criteria

Все 10 пунктов имеют исполняемое acceptance-покрытие; существующие сценарии не удалены; @spec:главный-workflow отсутствует; раздел удалён из SPEC.md; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T14:19:53Z**

Перенос завершён: пункты 1–2 закреплены в workflow_tools/lifecycle/run_lock; 3–4 — graph_execution/agent_types/control_endpoint; 5–6 — artifact_completion; 7 — process_execution с новой проверкой отсутствия partial artifacts; 8 — lifecycle/control_endpoint/run_lock/recovery; 9–10 — lifecycle/process_execution/recovery. Существующие сценарии не удалялись. Полный ./scripts/test.sh и ./scripts/build.sh прошли.
