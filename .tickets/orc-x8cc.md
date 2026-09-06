---
id: orc-x8cc
status: closed
deps: []
links: []
created: 2026-09-06T13:09:48Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести создание и восстановление Agent attempt из SPEC.md в acceptance features

Перенести наблюдаемый контракт commit создания, глобальной нумерации, recovery-матрицы и crash leftovers в тематические Cucumber Rules; добавить недостающие сценарии, затем удалить секцию и её source-теги.

## Acceptance Criteria

Все нормативные правила секции Создание и восстановление Agent attempt подтверждены acceptance-сценариями; секция удалена из SPEC.md; @spec:создание-и-восстановление-agent-attempt отсутствует; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T13:09:54Z**

Начат аудит секции Создание и восстановление Agent attempt против recovery, lifecycle и graph execution features и фактической storage/scheduler реализации.

**2026-09-06T13:21:01Z**

Полностью перенесена секция Создание и восстановление Agent attempt: recovery.feature теперь явно фиксирует atomic existence, bootstrap attempt 0, незаполнение пропусков, резервирование orphan/crash номеров, validation и игнорирование leftovers; существующие Agent, Process, completion и /exit сценарии сохранены в своих capability features, source-теги удалены. Аудит выявил mismatch: resume отвергал run после crash между spec.yaml и initial attempt и scheduler начинал бы с 1. Loader теперь принимает пустую attempt-модель, scheduler восстанавливает bootstrap первого Step с пустым input и номером 0; добавлены Cucumber-сценарий и fault-injection unit-тест. ./scripts/test.sh — PASS; ./scripts/build.sh — PASS; git diff --check — PASS.
