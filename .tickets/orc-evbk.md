---
id: orc-evbk
status: closed
deps: []
links: []
created: 2026-09-01T13:14:56Z
type: feature
priority: 1
assignee: viktoraseev
tags: [run, inspection, cli, read-only, mvp]
---
# Показывать список durable runs

Добавить read-only команду orchestrator run list, которая перечисляет существующие runs и вычисляет их состояние только из durable-модели, не захватывая lifecycle и не запуская Agent.

## Design

Сначала закрепить CLI и формат вывода в cli.md, затем переиспользовать storage parsing и validation через отдельную read-only библиотечную операцию; строки сортируются по RunId, производный status на диск не записывается.

## Acceptance Criteria

Команда на пустом корне успешна и печатает пустой результат; валидные active, blocked и completed runs перечисляются в детерминированном порядке с WorkflowId и вычисленным состоянием; неизвестные файлы run root игнорируются по format contract, противоречивый run даёт диагностику без изменения файлов; API acceptance проверяет побайтовую неизменность durable state, process acceptance закрепляет CLI parsing/stdout/exit codes; cli.md и feature Rules с source tags обновлены; canonical checks проходят.


## Notes

**2026-09-01T13:24:11Z**

Добавлены validated read model и orchestrator run list: детерминированная сортировка, derived active/blocked/completed, пустой root и fail-fast без частичного stdout. Acceptance и canonical checks проходят.
