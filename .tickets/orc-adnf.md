---
id: orc-adnf
status: closed
deps: []
links: [orc-1xw2, orc-stgc, orc-kgke, orc-urzj, orc-8t3g]
created: 2026-09-01T13:28:45Z
type: feature
priority: 0
assignee: viktoraseev
tags: [run, inspection, storage, concurrency, read-only, correctness]
---
# Читать согласованный snapshot работающего run

Сделать все read-only inspection команды устойчивыми к атомарным публикациям supervisor между чтением spec, attempts и artifacts, чтобы валидный active run не давал ложную corruption error.

## Design

Storage boundary читает fingerprint durable entries до и после построения validated model; если snapshot изменился во время чтения, операция повторяется с ограниченным числом попыток, а неизменный противоречивый snapshot возвращает validation error; Run lock не берётся.

## Acceptance Criteria

Concurrent commit attempt record и artifacts наблюдается только как старый или новый целый validated snapshot; временное несовпадение файлов вызывает retry без diagnostics и partial stdout; стабильно противоречивая модель возвращает 3; число retry ограничено и при непрерывных изменениях возвращается документированная runtime error; list, show, artifacts, JSON и watch используют одну границу; детерминированные tests управляют commit barriers без sleep; durable bytes и Run lock не меняются; SPEC.md, cli.md и source-tagged acceptance обновлены; canonical checks проходят.

