---
id: orc-trvr
status: closed
deps: [orc-z4o4]
links: [orc-i60z, orc-oijv]
created: 2026-09-01T14:33:37Z
type: feature
priority: 2
assignee: viktoraseev
tags: [workflow, graph, dependencies, show, read-only, json]
---
# Показывать dependency graph source workflow

Добавить orchestrator workflow graph <workflow-id> [--format text|json], чтобы видеть topology source workflow без чтения config, prompt templates или создания run.

## Design

Команда использует выбранный source workflow и отдельную graph-validation boundary: проверяет source schema, уникальность Steps, существование depends-on references и статическую reachability, после чего строит typed graph с nodes в source order, dependency edges в source order и явно отмеченной bootstrap initial activation первого Step; Agent и Prompt references не разрешаются.

## Acceptance Criteria

Text печатает workflow header, bootstrap Step и edges <dependency> -> <step>, JSON возвращает object {workflow,path,bootstrap,nodes,edges}; isolated non-entry Step, неизвестная dependency и недостижимый cycle дают 3 без stdout; неизвестный workflow даёт 4, невалидный ID/format — 2 до чтения source; команда не читает config/prompts и ничего не меняет; workflow.spec.md, cli.md и source-tagged API/process acceptance обновлены; canonical checks проходят.


## Notes

**2026-09-01T14:44:49Z**

Реализованы workflow graph text/JSON и typed WorkflowGraph с bootstrap, nodes и edges; source topology проверяется без чтения config/prompts, ошибки reachability/dependency не дают stdout; добавлены docs и acceptance. ./scripts/test.sh и ./scripts/build.sh проходят.
