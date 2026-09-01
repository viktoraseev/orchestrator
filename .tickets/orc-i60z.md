---
id: orc-i60z
status: closed
deps: [orc-p97s]
links: [orc-trvr, orc-oijv]
created: 2026-09-01T14:33:37Z
type: feature
priority: 1
assignee: viktoraseev
tags: [workflow, validate, bulk, report, read-only, json]
---
# Проверять все source workflows одним отчётом

Добавить orchestrator validate --all [--format text|json], чтобы проверить весь workflow catalog одной read-only командой и получить полный отчёт вместо последовательного ручного запуска validate.

## Design

Команда сначала детерминированно открывает полный workflow catalog, затем через существующую preflight/materialization boundary независимо проверяет каждый WorkflowId в сортированном порядке, продолжая после validation/not-found ошибок отдельного кандидата; никаких runs и source-файлов не создаёт и не меняет.

## Acceptance Criteria

--all взаимоисключаем с позиционным WorkflowId; text содержит workflow <id>: valid либо workflow <id>: invalid: <diagnostic>, JSON имеет форму {workflows:[{workflow,valid,diagnostics}]}; пустой catalog успешен; все valid дают 0, наличие invalid даёт 3 только после полного stdout, глобальная I/O/runtime ошибка даёт 1 без partial stdout, невалидный basename contract file даёт 3 без partial stdout; порядок стабилен; SPEC.md, cli.md и source-tagged API/process acceptance обновлены; canonical checks проходят.


## Notes

**2026-09-01T14:44:49Z**

Реализованы validate --all text/JSON, typed WorkflowValidationReport, сортированный полный отчёт с кодом 3 после stdout, пустой catalog и fail-fast catalog errors; добавлены docs и source-tagged API/process acceptance. ./scripts/test.sh и ./scripts/build.sh проходят.
