---
id: orc-p97s
status: closed
deps: [orc-hqap]
links: []
created: 2026-09-01T09:04:48Z
type: feature
priority: 1
assignee: viktoraseev
tags: [validate, workflow, cli, mvp]
---
# Проверить явно выбранный workflow

Реализовать orchestrator validate <workflow-id> как законченный read-only вертикальный срез: разрешить явно указанный template, materialize выбранные Agents и prompt templates в памяти и выполнить полную validation из workflow.spec.md без создания run.

## Design

Добавить библиотечный validate entrypoint, который получает корень состояния и registry AgentType, а бинарник только разбирает аргументы и отображает результат. Представить WorkflowId, StepId, PromptId и InputId валидированными типами; сохранить порядок Steps; читать только выбранный workflow и используемые prompts. Validation должна охватывать schema, ссылки, placeholders, reachability и cycles одним детерминированным проходом без scheduler или storage run.

## Acceptance Criteria

- Валидный явно выбранный workflow печатает workflow <workflow-id>: valid в stdout и завершается с кодом 0 без создания или изменения run.
- Явный WorkflowId выбирается без чтения default-workflow и невыбранных workflow templates, но config читается для Agents и default-agent.
- Невалидный ID завершается с кодом 2, отсутствующий template с кодом 4, а невалидный config, workflow, Agent или prompt с кодом 3 и диагностикой error: в stderr.
- Полностью проверяются schema workflow и prompts, уникальность IDs, Agent type/model/reasoning и поддержка native resume, ссылки depends-on и prompt, placeholders, статическая достижимость и допустимые cycles.
- Ошибки выдаются детерминированно: структура файла, затем Steps в порядке workflow, затем ссылки и graph.
- features/ содержит Rule и сценарии для каждого нормативного правила раздела Validation с тегами @cli:validate, @cli:выбор-workflow, @workflow:validation, @format:workflow-template и @format:prompt-template.
- Основные сценарии вызывают публичный validate entrypoint; отдельные @process-сценарии проверяют CLI parsing, stdout, stderr и exit code без дублирования семантики.
- ./scripts/test.sh и ./scripts/build.sh завершаются успешно.


## Notes

**2026-09-01T09:22:54Z**

Реализованы explicit validate, materialization workflow/Agents/prompts, AgentRegistry injection, strict schema, placeholders, reachability/cycles и deterministic errors. Покрытие: 30 API + 3 process Cucumber scenarios. Проверки: ./scripts/test.sh, ./scripts/build.sh.
