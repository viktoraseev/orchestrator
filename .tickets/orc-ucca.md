---
id: orc-ucca
status: closed
deps: []
links: []
created: 2026-09-06T16:51:14Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести Workflow и Prompt templates из format-спеки

Перенести крупным блоком нормативные правила разделов Workflow template и Prompt template из format.spec.md в acceptance features, добавить недостающее исполняемое покрытие и удалить перенесённый текст и source-теги без удаления существующих сценариев.

## Acceptance Criteria

Все правила двух разделов представлены в Rule/Scenario features; недостающие schema, Process path и prompt contracts проверяются через публичные entrypoints; разделы удалены из format.spec.md и ссылки обновлены; существующие сценарии не удалены; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T16:59:22Z**

Перенесены разделы Workflow template и Prompt template: контракты распределены по workflow_validation, process_execution и source_catalog; добавлены acceptance-сценарии структуры YAML, materialization relative cwd/executable, невалидных Process paths/полей и YAML-похожего Markdown. Существующие сценарии не удалялись. Targeted tests workflow_validation, process_execution и source_catalog проходят.

**2026-09-06T17:01:23Z**

Финальная проверка: ./scripts/test.sh завершился успешно (format, Clippy, unit и полный Cucumber набор), ./scripts/build.sh собрал release-бинарь. Аудит git diff подтверждает отсутствие удалённых Scenario/Scenario Outline и отсутствие устаревших @format:workflow-template, @format:prompt-template, @format:workflow-yaml tags.
