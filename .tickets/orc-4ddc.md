---
id: orc-4ddc
status: closed
deps: [orc-evbk]
links: []
created: 2026-09-01T13:15:07Z
type: feature
priority: 1
assignee: viktoraseev
tags: [run, inspection, frontier, cli, read-only, mvp]
---
# Показывать подробное состояние одного run

Добавить orchestrator run show <run-id> с материализованным workflow, attempts, последними native sessions, completion и вычисленным frontier без запуска или resume.

## Design

Операция читает run через ту же storage validation boundary, что resume, и формирует typed read model; CLI только отображает его стабильными строками в порядке workflow и глобальных attempt numbers.

## Acceptance Criteria

Для валидного run вывод содержит RunId, WorkflowId, derived run state, каждый Step и его attempts в глобальном порядке, последнюю session ID, completion и ready/blocked dependencies; completed и циклические runs отображаются без запуска Agent; неизвестный RunId возвращает 4, невалидный durable run возвращает 3, занятый run остаётся доступен для read-only inspection; никакие durable или volatile файлы не создаются; API и process acceptance, cli.md и source-tagged Rules обновлены; canonical checks проходят.


## Notes

**2026-09-01T13:24:19Z**

Добавлен orchestrator run show: стабильные summary, Steps, attempts, last session, inputs и ready/missing frontier через тот же durable validator; занятый run читается без lifecycle lock.
