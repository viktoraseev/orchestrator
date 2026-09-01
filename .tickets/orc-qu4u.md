---
id: orc-qu4u
status: closed
deps: []
links: []
created: 2026-09-01T08:26:20Z
type: feature
priority: 1
assignee: viktoraseev
tags: [config, cli, mvp]
---
# Прочитать эффективную конфигурацию через config get и config list

Первый вертикальный срез проекта: инициализировать один Rust crate orchestrator и реализовать чтение эффективной конфигурации командами config get и config list. Поддержать ключи default-workflow, default-agent и max-parallel-agents, ORC_HOME и отсутствие config.yaml без чтения текущего каталога или пользовательского состояния в тестах.

## Design

Бинарник только разбирает аргументы и отображает результат библиотечного API. Библиотека разрешает корень состояния, читает и полностью валидирует config.yaml по format.spec.md, затем возвращает типизированное представление эффективных значений. Acceptance-контракт оформить в features/config.feature с тегами источников; сценарии вывода, exit code и CLI parsing выполнить в @process-режиме с уникальным ORC_HOME.

## Acceptance Criteria

- config get печатает значение выбранного поддерживаемого ключа, а для отсутствующего файла возвращает null, null и 5 согласно cli.md.
- config list печатает три строки в фиксированном порядке и не перечисляет Agents или workflow-файлы.
- Относительный ORC_HOME и невалидный config завершаются с кодом 3 и диагностикой error: в stderr.
- Неизвестный ключ и неверное число аргументов завершаются с кодом 2.
- Cucumber-сценарии содержат Rule и теги @cli:config-get-config-set-и-config-list и @format:config-yaml; локальные parser/schema edge cases покрыты unit-тестами.
- ./scripts/test.sh и ./scripts/build.sh завершаются успешно.


## Notes

**2026-09-01T08:32:48Z**

Реализованы config get/list, строгая schema validation и process-Cucumber. Проверки: ./scripts/test.sh, ./scripts/build.sh.
