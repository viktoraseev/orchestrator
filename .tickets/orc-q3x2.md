---
id: orc-q3x2
status: closed
deps: [orc-jutm]
links: [orc-ltal, orc-jutm]
created: 2026-09-01T13:47:52Z
type: feature
priority: 2
assignee: viktoraseev
tags: [agent, catalog, config, cli, read-only, json]
---
# Перечислять named Agents из config

Добавить `orchestrator agent list [--format text|json]`, чтобы пользователь видел все настроенные AgentId и их materialized-independent параметры.

## Design

Переиспользовать typed catalog renderer из `workflow list`, но читать config через общую validation границу; descriptors содержат AgentId, type, model и reasoning и сортируются по AgentId независимо от YAML mapping order.

## Acceptance Criteria

Отсутствующий config или пустой `agents` mapping даёт успешный пустой результат; text печатает `agent <id>: type=<type> model=<model> reasoning=<reasoning>`, JSON — deterministic array typed descriptors; весь config проверяется до вывода, поэтому неизвестные поля или невалидный Agent дают code 3 без partial stdout; команда не применяет `default-agent` и ничего не изменяет; `cli.md`, `format.spec.md` и source-tagged API/process acceptance обновлены; canonical checks проходят.
