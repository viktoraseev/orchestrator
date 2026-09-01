---
id: orc-9tas
status: closed
deps: [orc-jlen]
links: []
created: 2026-09-01T10:08:05Z
type: feature
priority: 1
assignee: viktoraseev
tags: [scheduler, workflow, artifact, prompt, lifecycle, mvp]
---
# Выполнить линейный workflow с artifact input

Расширить lifecycle с single-step до законченного линейного workflow: после durable completion source Step вычислить ready target, атомарно создать следующий attempt с зафиксированным input, сформировать prompt из выбранных artifacts и последовательно выполнять Steps до terminal completion.

## Design

Выделить graph frontier и создание attempt внутри lifecycle API, сохранив storage единственным читателем/писателем durable run. В этом PR scheduler обслуживает один запускаемый attempt за раз и линейную dependency group из одного source; attempt numbers остаются глобальными. AgentRunRequest получает неизменяемый input mapping с ключом (source-step-id, input-id), а prompt renderer заменяет path/content placeholders из того же mapping. Resume сначала полностью восстанавливает и валидирует все опубликованные attempts линейного run.

## Acceptance Criteria

- Completion source attempt запускает следующий scheduling pass и создаёт target attempt только после terminal completed source record.
- Новый attempt получает следующий глобальный номер, input содержит номер выбранного source attempt в порядке depends-on и после публикации не изменяется.
- AgentType получает mapping (source-step-id, input-id) на durable artifact path; одинаковые InputId разных Steps не смешиваются типами или строковыми ключами.
- Placeholder path заменяется абсолютным durable path, placeholder content — точными UTF-8 bytes выбранного artifact, а materialized prompt null формирует пустую строку.
- Невалидный UTF-8 artifact для content fail-fast завершает lifecycle с кодом 3 до создания target attempt и запуска Agent.
- Последующее изменение config, workflow и prompt templates не влияет на уже materialized run и его resume.
- Завершение terminal Step оставляет всю историю attempts/artifacts, печатает run <run-id>: completed и Run <run-id> exited с кодом 0.
- Resume валидирует spec, все attempt records, inputs и artifacts линейного run до Agent-вызовов и durable-записей; неполная, старая или противоречивая input group возвращает 3.
- API acceptance-сценарии проверяют журнал fake AgentType, input mapping, prompt и durable state; process-сценарий подтверждает последовательный запуск через ORC_AGENT_COMMAND без дублирования graph-семантики.
- Rule и сценарии трассируются тегами @workflow:initial-activation-dependencies-и-frontier, @workflow:inputs-и-prompt, @workflow:fail-fast-input-validation, @spec:создание-и-восстановление-agent-attempt и @format:agent-attempt-record.
- ./scripts/test.sh и ./scripts/build.sh завершаются успешно.


## Notes

**2026-09-01T10:36:42Z**

Реализован линейный scheduler: immutable input mapping (StepId, InputId), durable source attempt numbers, path/content prompt rendering, fail-fast UTF-8 validation и полное resume-восстановление. Acceptance: graph_execution.feature. Проверки: ./scripts/test.sh, ./scripts/build.sh, cargo fmt --all --check.
