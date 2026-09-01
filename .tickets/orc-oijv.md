---
id: orc-oijv
status: closed
deps: [orc-p97s]
links: [orc-trvr, orc-i60z]
created: 2026-09-01T14:33:37Z
type: feature
priority: 1
assignee: viktoraseev
tags: [workflow, plan, materialization, preflight, read-only, json]
---
# Показывать materialized execution plan без создания run

Добавить orchestrator workflow plan <workflow-id> [--format text|json], чтобы заранее увидеть эффективные Agents, prompts и parallel limit именно в том виде, который был бы зафиксирован start.

## Design

Команда вызывает ту же полную preflight/materialization boundary, что explicit start после выбора WorkflowId, но останавливается до резервирования RunId и публикации spec; typed plan содержит WorkflowId, эффективный max-parallel-agents и Steps в source order с полностью materialized Agent, prompt content, human, depends-on и outputs.

## Acceptance Criteria

Text детерминированно показывает workflow, effective parallel limit и summary каждого Step; JSON возвращает один object {workflow_id,max_parallel_agents,steps} с effective Agent {id,type,model,reasoning}, prompt string/null и source-order arrays; любые config/workflow/prompt/graph ошибки дают 3 без stdout, отсутствующий template — 4, невалидный ID/format — 2; команда не создаёт run, lock или control endpoint и не меняет source; SPEC.md, workflow.spec.md, format.spec.md, cli.md и source-tagged API/process acceptance обновлены; canonical checks проходят.


## Notes

**2026-09-01T14:35:54Z**

Уточнение реализации: действующий format.spec.md прямо запрещает AgentId в materialized workflow, поэтому workflow plan представляет тот же кандидат start с Agent {type,model,reasoning} без AgentId; это сохраняет единую materialization boundary и не меняет durable schema.

**2026-09-01T14:44:49Z**

Реализованы workflow plan text/JSON и typed WorkflowPlan через ту же materialize_for_lifecycle boundary, что start; plan не создаёт run и следует существующей schema без AgentId; добавлены docs и acceptance. ./scripts/test.sh и ./scripts/build.sh проходят.
