---
id: orc-fw88
status: open
deps: []
links: []
created: 2026-09-01T13:56:11Z
type: feature
priority: 2
assignee: viktoraseev
tags: [agent, catalog, show, config, cli, read-only, json]
---
# Показывать одного named Agent

Добавить orchestrator agent show <agent-id> [--format text|json], чтобы получить параметры конкретного Agent после agent list.

## Design

Команда парсит AgentId на CLI boundary, полностью проверяет config общей boundary и только затем выбирает typed Agent descriptor; отдельной partial config validation и применения default-agent нет.

## Acceptance Criteria

Существующий Agent выводит type, model и reasoning; text имеет форму agent <id>: type=<type> model=<model> reasoning=<reasoning>, JSON — object с полями agent, type, model и reasoning; неизвестный Agent возвращает 4, невалидный ID/format — 2 до чтения config, любая config corruption — 3 без stdout; source state не меняется; cli.md, format.spec.md и source-tagged API/process acceptance обновлены; canonical checks проходят.

