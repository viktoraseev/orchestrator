---
id: orc-btu0
status: closed
deps: [orc-qu4u]
links: []
created: 2026-09-01T08:26:33Z
type: feature
priority: 1
assignee: viktoraseev
tags: [config, cli, mvp]
---
# Атомарно настроить max-parallel-agents

Реализовать orchestrator config set max-parallel-agents <positive-integer> как минимальную изменяющую состояние возможность. Команда должна сохранять остальные поля существующего config.yaml, публиковать целый валидный файл атомарно и немедленно отражаться в config get и config list.

## Design

Расширить библиотечную config/storage-границу операцией read-modify-validate-publish. Кандидат полностью проверяется до записи, временный файл размещается рядом с config.yaml, durable commit выполняется атомарной заменой. Конкурентные успешные set не должны смешивать части файлов; наблюдаемое значение определяется последним durable commit.

## Acceptance Criteria

- Положительное целое сохраняется и команда печатает max-parallel-agents: <value> с кодом 0.
- Ноль, отрицательное значение и нецелое значение завершаются с кодом 3 и оставляют bytes прежнего config.yaml неизменными.
- Неизвестный ключ и неверное число аргументов завершаются с кодом 2 без изменения config.yaml.
- Существующие default-workflow, default-agent и agents сохраняются без семантического изменения после успешной записи.
- Два конкурентных успешных set оставляют целый валидный config.yaml со значением одного из завершённых кандидатов.
- Acceptance-сценарии имеют Rule с тегами @cli:config-get-config-set-и-config-list и @format:config-yaml; внутренняя атомарная публикация и fault injection покрыты unit-тестами.
- ./scripts/test.sh и ./scripts/build.sh завершаются успешно.


## Notes

**2026-09-01T08:35:03Z**

Реализованы валидированный set, атомарная durable-публикация, конкурентный process-сценарий и fault injection до commit. Проверки: ./scripts/test.sh, ./scripts/build.sh.
