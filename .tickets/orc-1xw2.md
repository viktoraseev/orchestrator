---
id: orc-1xw2
status: closed
deps: [orc-kgke]
links: [orc-stgc, orc-kgke, orc-urzj, orc-adnf, orc-8t3g]
created: 2026-09-01T13:27:01Z
type: feature
priority: 1
assignee: viktoraseev
tags: [run, inspection, watch, cli, signals, read-only]
---
# Наблюдать изменения active run

Добавить orchestrator run watch <run-id>, который read-only наблюдает durable snapshots active run и завершает ожидание при blocked или completed состоянии.

## Design

Переиспользовать typed inspection model и его fingerprint; команда сразу публикует initial snapshot, затем с фиксированным интервалом перечитывает run и выводит только изменившиеся validated snapshots, не получая lifecycle lock; text и JSON используют renderers предыдущего PR.

## Acceptance Criteria

Initial snapshot выводится немедленно; неизменившееся состояние не дублируется; новые session activations, attempts, completions и frontier дают один новый snapshot после полного durable commit; completed и blocked initial или последующий snapshot завершают команду с 0; active run ожидается до изменения или SIGINT/SIGTERM/SIGHUP и возвращает документированный signal code; transient атомарные temporary files игнорируются, невалидная durable-модель fail-fast с 3 без частичного нового snapshot; watch не создаёт файлов, endpoint или Agent process и не мешает supervisor lock; детерминированные API/process acceptance используют polling с deadline без sleep-синхронизации; cli.md и source tags обновлены; canonical checks проходят.

