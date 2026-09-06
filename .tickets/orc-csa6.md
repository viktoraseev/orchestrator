---
id: orc-csa6
status: closed
deps: []
links: []
created: 2026-09-04T19:13:00Z
type: task
priority: 2
assignee: viktoraseev
tags: [rust, refactoring, domain]
---
# Выделить доменные агрегаты в src/domain

Перенести перемешанные внутренние представления workflow, run и agent attempt в отдельный src/domain, сохранив связанные типы и core-методы рядом с сущностями. Оставить parsing/validation workflow, lifecycle scheduling, storage I/O, process adapters и CLI read models в текущих boundary-модулях. Публичный API и продуктовое поведение не менять.

## Design

domain/workflow.rs владеет Workflow, Step, ProcessStep, WorkflowId и SymbolicId; domain/run.rs владеет materialized run aggregate; domain/attempt.rs владеет AttemptRecord, AttemptEvent и DurableAttempt. src/domain.rs селективно re-export'ит crate-internal API. Не создавать пустые Agent/Artifact сущности до появления собственных инвариантов.

## Acceptance Criteria

В src/domain находятся агрегаты workflow/run/attempt вместе с конструкторами, преобразованиями и event-query методами; src/workflow.rs и src/run.rs больше не объявляют эти сущности; внешние импорты orchestrator не меняются; сериализованный durable-формат не меняется; новые visibility не шире pub(crate); cargo fmt, ./scripts/test.sh, ./scripts/build.sh и git diff --check успешны.


## Notes

**2026-09-04T19:22:04Z**

Уточнённая граница после карты зависимостей: domain/agent.rs также владеет Agent/AgentId, а единый domain WorkflowId заменяет дубликаты в config.rs и workflow.rs. AttemptRecord инкапсулирует создание и append-only события и проверяет порядок истории; MaterializedWorkflow инкапсулирует structural validation. CLI/read models, filesystem storage, scheduler и process adapters остаются снаружи. Публичные config::AgentId, config::WorkflowId, workflow::WorkflowId, run::RunId и crate::RunId сохранены re-export'ами.

**2026-09-04T19:23:55Z**

Завершено: добавлен src/domain с feature-oriented модулями agent, workflow, run и attempt. Перенесены Agent/AgentId, единый WorkflowId/SymbolicId, Workflow/Step/ProcessStep, RunId/materialized run и AttemptRecord/DurableAttempt. Вместе с сущностями перенесены serde-схемы, ID validation, materialized run validation, append-only event mutations и validation истории attempt. Старые публичные пути сохранены селективными re-export; RawAgent и дубликат WorkflowId удалены. Продуктовое поведение и durable format не менялись, поэтому спецификации/features не правились. Успешны cargo check, cargo test --lib, cargo fmt --all -- --check, ./scripts/test.sh, ./scripts/build.sh и git diff --check.
