---
id: orc-cl8v
status: closed
deps: []
links: []
created: 2026-09-06T18:55:12Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести остаток format-спеки и удалить документ

Перенести крупным блоком все оставшиеся нормативные правила format.spec.md: выбор state root, durable layout, kernel lock file, volatile entries и общую config YAML validation — в тематические acceptance features; убрать дубли root-контракта из cli.md, удалить source-теги и сам format.spec.md без удаления существующих сценариев.

## Acceptance Criteria

Default HOME/.orc и отсутствие current-directory fallback подтверждены process-сценарием; config root mapping, duplicate/unknown fields, source/run paths, arbitrary active.lock contents и volatile leftovers имеют исполняемые примеры; все @format:* и перенесённые @cli:корень-состояния-и-переменные-окружения удалены; format.spec.md удалён, ссылки актуальны; существующие Rule/Scenario не удалены; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T19:01:11Z**

Перенесён весь остаток format.spec.md по тематическим Rules без технического mega-scenario. Добавлены два недостающих примера: unset ORC_HOME выбирает только HOME/.orc и игнорирует current-directory config; unlocked active.lock принимает arbitrary bytes. Config outline расширен root YAML sequence. Точные source/run paths закреплены в Rule descriptions при существующих executable examples; @format:* и source-теги удалённого cli root-раздела сняты, @process сохранены. Targeted config и lifecycle Cucumber проходят без undefined/ambiguous steps.

**2026-09-06T19:02:36Z**

Финальная проверка: cargo test --test config, cargo test --test lifecycle, ./scripts/test.sh и ./scripts/build.sh проходят; новые scenarios выбраны и выполнены без undefined/ambiguous steps. Gherkin diff-аудит подтверждает: Rule/Scenario/Scenario Outline не удалялись, все @process сохранены, сняты только @format:* и @cli:корень-состояния-и-переменные-окружения удалённых источников. format.spec.md удалён, активных ссылок на него нет, git diff --check чист.
