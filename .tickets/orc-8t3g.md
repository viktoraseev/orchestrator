---
id: orc-8t3g
status: closed
deps: [orc-kgke]
links: [orc-1xw2, orc-stgc, orc-kgke, orc-urzj, orc-adnf]
created: 2026-09-01T13:28:59Z
type: feature
priority: 2
assignee: viktoraseev
tags: [run, inspection, filter, cli, read-only]
---
# Фильтровать список durable runs

Добавить к orchestrator run list повторяемые read-only фильтры --state active|blocked|completed и --workflow <workflow-id> для рабочих корней с большим числом runs.

## Design

Фильтры применяются к полностью validated typed summaries после детерминированной сортировки; text и JSON renderers получают один и тот же отфильтрованный список, а validation не пропускается для скрытых runs.

## Acceptance Criteria

Один и несколько --state объединяются как OR, workflow с state как AND; повтор filters идемпотентен; порядок совпадает с unfiltered list; пустой результат успешен и соответствует выбранному format; невалидный state или WorkflowId даёт code 2 без чтения runs; противоречивый скрытый run по-прежнему даёт 3, чтобы filter не маскировал corruption; cli.md и source-tagged API/process acceptance обновлены; canonical checks проходят.

