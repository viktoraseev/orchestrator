---
id: orc-kgke
status: closed
deps: [orc-stgc]
links: [orc-1xw2, orc-stgc, orc-urzj, orc-adnf, orc-8t3g]
created: 2026-09-01T13:26:45Z
type: feature
priority: 1
assignee: viktoraseev
tags: [run, inspection, json, cli, automation]
---
# Выводить run inspection в JSON

Добавить --format text|json к orchestrator run list, run show и run artifacts с единым документированным JSON read model для скриптов.

## Design

Сделать typed inspection model библиотечным результатом и отделить его от text/JSON renderers; text остаётся побайтово совместимым, JSON сериализуется одним документом только после полной validation без смешивания diagnostics.

## Acceptance Criteria

Формат по умолчанию text полностью сохраняется; --format json для list возвращает array runs, для show один run со Steps, attempts, sessions и frontier, для artifacts array descriptors; числа остаются JSON numbers, отсутствующие значения null, порядок arrays детерминирован; schema и совместимость закреплены в cli.md, неизвестный format даёт CLI code 2, ошибки оставляют stdout пустым; API acceptance проверяет typed model, process acceptance оба renderer; canonical checks проходят.

