---
id: orc-ni36
status: closed
deps: []
links: []
created: 2026-09-06T15:02:49Z
type: task
priority: 2
assignee: viktoraseev
---
# Завершить перенос SPEC.md в acceptance features

Перенести оставшийся контракт read-only inspection из SPEC.md в исполняемые feature-сценарии, удалить source-теги и устаревшие ссылки, затем удалить SPEC.md.

## Acceptance Criteria

Все наблюдаемые правила read-only inspection подтверждены features/run_inspection.feature; существующие сценарии сохранены; SPEC.md и ссылки на него удалены; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T15:09:29Z**

Перенесён весь оставшийся контракт read-only inspection в features/run_inspection.feature: согласованный snapshot и validation, вычисляемые status/session, watch initial/change/volatile entries/SIGTERM, verify с явным RunId. Существующие сценарии сохранены, @spec:read-only-inspection и ссылки удалены, SPEC.md удалён. Проверки: cargo test --test run_inspection (26/26), ./scripts/test.sh, ./scripts/build.sh, git diff --check.
