---
id: orc-p7ve
status: closed
deps: []
links: []
created: 2026-09-06T12:21:03Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести Native resume из SPEC.md в acceptance features

Перенести весь наблюдаемый контракт раздела Native resume в тематические Rule и Cucumber-сценарии, затем удалить раздел и его source-теги из SPEC.md.

## Acceptance Criteria

Все правила Native resume закреплены исполняемыми сценариями; раздел Native resume удалён из SPEC.md; @spec:native-resume отсутствует; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T12:21:08Z**

Начат аудит раздела Native resume: требования раскладываются по lifecycle, process execution, workflow validation, human execution и shutdown; после подтверждения сценариями раздел и source-теги будут удалены.

**2026-09-06T12:30:25Z**

Перенесён весь контракт Native resume: добавлены отдельные Rules и сценарии для обязательного RunId, отсутствия auto-retry, выбора/отсутствия session activation, fail-fast несовместимого Agent type на start/resume, at-least-once Process и TTY/user shutdown; раздел удалён из SPEC.md, @spec:native-resume удалены. Проверки: cargo test --test lifecycle, cargo test --test process_execution, ./scripts/test.sh, ./scripts/build.sh, git diff --check — успешно.
