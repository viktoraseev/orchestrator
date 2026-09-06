---
id: orc-kx8r
status: closed
deps: []
links: []
created: 2026-09-06T13:59:06Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести completion и artifacts из SPEC.md в acceptance features

Перенести наблюдаемый контракт source artifacts, validation полного completion-кандидата, commit после возврата Agent и recovery до commit в тематический Cucumber feature; сохранить существующие сценарии, добавить отсутствующие границы и удалить секцию/source-тег.

## Acceptance Criteria

Все нормативные правила секции «Публикация artifacts и completion» подтверждены acceptance-сценариями; секция удалена из SPEC.md; @spec:публикация-artifacts-и-completion отсутствует; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T13:59:16Z**

Начат аудит полного раздела против lifecycle, control endpoint, recovery features и storage/control реализации. План: сохранить переносом существующие сценарии и добавить только отсутствующие наблюдаемые границы.

**2026-09-06T14:04:27Z**

Перенесены 12 существующих Cucumber executions из lifecycle/control endpoint в features/artifact_completion.feature без изменения шагов; добавлены 3 отсутствующих сценария: empty outputs, external source через absolute symlink и потеря volatile candidate при SIGKILL до возврата Agent. Раздел удалён из SPEC.md, source-тег удалён, Rustdoc AttemptControl обновлён. Расхождений с реализацией не обнаружено. Проверки cargo test --test lifecycle, ./scripts/test.sh, ./scripts/build.sh и git diff --check прошли.
