---
id: orc-stgc
status: closed
deps: [orc-adnf]
links: [orc-1xw2, orc-kgke, orc-urzj, orc-adnf, orc-8t3g]
created: 2026-09-01T13:26:36Z
type: feature
priority: 1
assignee: viktoraseev
tags: [run, artifact, inspection, cli, read-only]
---
# Перечислять опубликованные artifacts run

Добавить orchestrator run artifacts <run-id>, чтобы пользователь мог обнаружить все опубликованные durable artifacts до вызова run artifact.

## Design

Выделить из inspection внутренней модели typed artifact descriptors; после полной validation перечислять только artifacts completed attempts в порядке глобального attempt number и outputs materialized Step, не сканируя orphan files как результат.

## Acceptance Criteria

Пустой набор успешен; каждая строка содержит attempt number, StepId, InputId, размер bytes и абсолютный durable path; несколько версий одного output остаются отдельными строками в порядке attempt; orphan, temporary и artifacts незавершённых attempts не выводятся; неизвестный run возвращает 4, противоречивый run 3, ошибки не дают частичного stdout; операция read-only и доступна при удерживаемом Run lock; cli.md и source-tagged API/process acceptance обновлены; canonical checks проходят.

