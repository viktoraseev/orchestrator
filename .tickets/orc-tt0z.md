---
id: orc-tt0z
status: closed
deps: []
links: []
created: 2026-09-08T07:46:28Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести Source catalogs из cli.md в acceptance features

Пачка: весь раздел Source catalogs в cli.md — read-only list catalogs, точные workflow show/plan renderers для Agent и Process Steps, expression renderers и полный preflight plan. Сначала усилить acceptance-покрытие CLI-грамматики list-команд, затем удалить дублирующий раздел и все @cli:source-catalogs tags, сохранив остальные source tags.

## Acceptance Criteria

Acceptance-сценарии явно покрывают разрешённые аргументы workflow/agent/prompt list; каждое правило удаляемого раздела сопоставлено с сохраняемым Rule/Scenario; раздел Source catalogs и только его @cli tags удалены; targeted Cucumber, scripts/test.sh, scripts/build.sh и git diff --check успешны.


## Notes

**2026-09-08T07:59:06Z**

Перенесены 4 нормативных пункта раздела Source catalogs. Добавлен Rule для CLI-грамматики list-команд (6 process examples), усилена точная JSON schema Agent workflow show, добавлены process-сценарии точных text/JSON workflow show и text workflow plan, а Agent plan text теперь сравнивается целиком. Удалён раздел Source catalogs и все 9 тегов @cli:source-catalogs; остальные теги сохранены. Targeted Cucumber: conditional_graph 27/27, process_execution 36/36, source_catalog 42/42, workflow_tools 15/15. ./scripts/test.sh, ./scripts/build.sh и git diff --check успешны.
