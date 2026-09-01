---
id: orc-urzj
status: closed
deps: [orc-adnf, orc-kgke]
links: [orc-1xw2, orc-stgc, orc-kgke, orc-adnf, orc-8t3g]
created: 2026-09-01T13:29:18Z
type: feature
priority: 1
assignee: viktoraseev
tags: [run, inspection, verify, diagnostics, cli, read-only]
---
# Проверять здоровье durable runs без resume

Добавить orchestrator run verify [run-id], который read-only проверяет один или все runs и выдаёт полный детерминированный отчёт без запуска Agent.

## Design

Переиспользовать consistent inspection snapshot, но собирать независимые diagnostics по runs вместо остановки на первой ошибке; renderer поддерживает text и JSON schema, а exit code определяется после формирования полного отчёта.

## Acceptance Criteria

Без RunId проверяются все числовые run directories по порядку, с RunId только выбранный; валидные runs получают valid, противоречивые — все независимо наблюдаемые diagnostics в стабильном порядке storage rules; наличие хотя бы одного invalid run даёт code 3 после полного отчёта, I/O failure code 1, неизвестный явный run code 4; active и locked runs проверяются через согласованный snapshot без lock; verify ничего не создаёт и не меняет, text/JSON семантически эквивалентны; SPEC.md, cli.md и source-tagged API/process acceptance обновлены; canonical checks проходят.

