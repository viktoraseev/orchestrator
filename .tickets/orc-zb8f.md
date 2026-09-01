---
id: orc-zb8f
status: closed
deps: [orc-btu0]
links: []
created: 2026-09-01T08:26:47Z
type: feature
priority: 1
assignee: viktoraseev
tags: [config, workflow, cli, mvp]
---
# Настроить workflow по умолчанию

Реализовать orchestrator config set default-workflow <workflow-id>. Возможность связывает конфигурацию с существующим workflow template, но не materialize и не валидирует сам workflow до команд start или validate.

## Design

Использовать общую атомарную config/storage-операцию. До построения кандидата проверить формат WorkflowId и наличие regular file workflow/<workflow-id>.yaml только под выбранным корнем состояния. После этого заменить одно поле, полностью проверить кандидат config и атомарно опубликовать его, сохранив остальные значения.

## Acceptance Criteria

- Для валидного ID существующего regular workflow-файла команда сохраняет default-workflow и печатает default-workflow: <workflow-id> с кодом 0.
- Отсутствующий workflow завершается с кодом 4, пишет error: в stderr и не изменяет config.yaml.
- Невалидный WorkflowId завершается с кодом 2 до записи состояния.
- Команда не читает и не валидирует содержимое выбранного workflow, prompt templates или невыбранные workflow-файлы.
- Успешная запись сохраняет default-agent, max-parallel-agents и agents, а config get default-workflow возвращает новое значение.
- Acceptance-сценарии имеют Rule с тегами @cli:config-get-config-set-и-config-list, @cli:корень-состояния-и-переменные-окружения и @format:workflow-template.
- ./scripts/test.sh и ./scripts/build.sh завершаются успешно.

## Notes

**2026-09-01T08:36:56Z**

Реализованы валидированный WorkflowId, проверка существования regular workflow-файла без чтения содержимого и атомарное сохранение default-workflow. Проверки: ./scripts/test.sh, ./scripts/build.sh.
