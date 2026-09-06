---
id: orc-rykb
status: closed
deps: []
links: []
created: 2026-09-06T17:03:24Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести materialized workflow из format-спеки

Перенести крупным блоком нормативные правила раздела Materialized workflow из format.spec.md в acceptance features, добавить недостающее исполняемое покрытие и удалить перенесённый текст и source-теги без удаления существующих сценариев.

## Acceptance Criteria

Все правила раздела представлены в Rule/Scenario features; durable schema, точные parameters, отсутствие mutable source references, candidate plan и порядок публикации проверяются через публичные entrypoints; раздел удалён из format.spec.md и ссылки обновлены; существующие сценарии не удалены; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T17:11:36Z**

Раздел Materialized workflow удалён из format.spec.md, требования распределены по lifecycle, workflow_tools и process_execution. Добавлены executable-проверки закрытой durable YAML schema Agent/Process Steps, точных пустого и NUL parameters, закрытой JSON plan schema без runtime values и независимости resume от изменённых config/workflow/prompt. Старые сценарии не удалялись; targeted lifecycle/process_execution/workflow_tools tests проходят.

**2026-09-06T17:12:38Z**

Финальная проверка: ./scripts/test.sh успешно прошёл format, Clippy, unit и полный Cucumber набор; ./scripts/build.sh собрал release-бинарь. Аудит подтверждает отсутствие удалённых Scenario/Scenario Outline, устаревших @format:materialized-workflow tags и ссылки cli.md на удалённую schema.
