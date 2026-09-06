---
id: orc-1ivx
status: closed
deps: []
links: []
created: 2026-09-06T19:36:19Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести контракт agent-facing CLI-команд из cli.md в features

Перенести крупный связный блок требований attempt complete и session activate в исполняемые feature-сценарии, подтвердить полноту покрытия и удалить ставший дублирующим раздел Agent-facing команды из cli.md вместе с его source-тегами.

## Acceptance Criteria

Все наблюдаемые требования раздела Agent-facing команды представлены Rule/Scenario в features; существующие сценарии и process-режимы не потеряны; раздел удалён из cli.md; затронутые Cucumber-сценарии, ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T19:42:34Z**

Перенесён полный agent-facing контракт: удалены команды и 12 требований раздела Agent-facing команды, а также 3 дублирующих требования из кодов завершения. Существующие Rules в artifact_completion.feature и lifecycle.feature сохраняют completion/session contracts; control_endpoint.feature дополнен Scenario Outline из 2 examples для запрета явных RunId/attempt selectors. Source-тег @cli:session-activate-и-attempt-complete удалён вместе с источником, scenarios не удалялись. Проверки: cargo test --test lifecycle; ./scripts/test.sh (exit 0); ./scripts/build.sh (exit 0); git diff --check.

**2026-09-06T19:42:45Z**

Уточнение объёма: в самом разделе Agent-facing команды было 13 нормативных пунктов; вместе с 2 пунктами списка команд и 3 дублирующими пунктами кодов завершения из cli.md удалено 18 нормативных пунктов.
