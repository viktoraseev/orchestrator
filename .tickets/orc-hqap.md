---
id: orc-hqap
status: closed
deps: [orc-zb8f]
links: []
created: 2026-09-01T09:04:24Z
type: feature
priority: 1
assignee: viktoraseev
tags: [config, agent, cli, mvp]
---
# Настроить Agent по умолчанию

Реализовать orchestrator config set default-agent <agent-id> и завершить публичный набор config set для поддерживаемых ключей. Команда выбирает только существующий named Agent из config.yaml, полностью проверяет кандидат config и атомарно сохраняет остальные значения.

## Design

Добавить валидированный AgentId и команду библиотечной config-границы, переиспользовав существующую read-modify-validate-publish операцию. Проверка существования выполняется по mapping agents уже прочитанного кандидата; запуск Agent type, чтение workflow и изменение runs в эту возможность не входят.

## Acceptance Criteria

- Существующий валидный Agent выбирается, команда печатает default-agent: <agent-id> с кодом 0, а config get и config list показывают новое значение.
- Отсутствующий Agent завершается с кодом 4 и оставляет config.yaml побайтово неизменным.
- Невалидный AgentId завершается с кодом 2 до публикации состояния.
- Невалидный type, model или reasoning кандидата завершается с кодом 3 и не изменяет config.yaml.
- Успешная запись сохраняет default-workflow, max-parallel-agents и весь mapping agents без семантического изменения.
- Acceptance-сценарии имеют Rule с тегами @cli:config-get-config-set-и-config-list и @format:config-yaml; локальные границы AgentId и registry validation покрыты unit-тестами.
- ./scripts/test.sh и ./scripts/build.sh завершаются успешно.


## Notes

**2026-09-01T09:09:11Z**

Реализованы AgentId, config set default-agent, коды 2/3/4, атомарное сохранение и process/unit coverage. Проверки: ./scripts/test.sh, ./scripts/build.sh.
