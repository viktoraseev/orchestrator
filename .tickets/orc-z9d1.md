---
id: orc-z9d1
status: closed
deps: []
links: []
created: 2026-09-06T16:36:26Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести остаток workflow-спеки в acceptance features

Перенести крупным блоком оставшиеся требования cycles, terminal/blocked run, planning, validation и fail-fast input validation из workflow.spec.md в исполняемые Rule/Scenario features; удалить дублирующий документ и его source-теги только после подтверждения покрытия.

## Acceptance Criteria

Каждое нормативное правило workflow.spec.md представлено в features и подтверждено acceptance-сценарием; существующие сценарии не удалены; workflow.spec.md и устаревшие ссылки удалены; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T16:46:21Z**

Перенесены все оставшиеся правила workflow.spec.md в feature Rules. Добавлены API acceptance-сценарии mixed cycle I/O, workflow-order готовых human Steps, incomplete/missing/unfinished/stale durable input; существующие сценарии не удалялись. workflow source-теги и устаревшие ссылки сняты, документ удалён. Точечный lifecycle target проходит.

**2026-09-06T16:47:34Z**

Проверки завершены: cargo test --test lifecycle, полный ./scripts/test.sh, ./scripts/build.sh и git diff --check проходят; аудит diff не нашёл удалённых Scenario/Scenario Outline или оставшихся workflow.spec.md/@workflow ссылок в product/test sources.
