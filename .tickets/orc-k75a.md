---
id: orc-k75a
status: closed
deps: []
links: []
created: 2026-09-06T09:23:19Z
type: task
priority: 2
assignee: viktoraseev
tags: [rust, refactoring, run]
---
# Разделить run boundary по ответственностям

Разделить перегруженный run boundary на дочерние feature-модули без изменения публичного API и продуктового поведения. Сохранить run/mod.rs публичным фасадом и владельцем lifecycle; вынести read-only inspection, durable storage, Unix control endpoint, выполнение attempt и scheduling в src/run/.

## Design

run/inspection.rs владеет inspection DTO и render; run/storage.rs — RunGuard, consistent snapshot, чтением/валидацией durable state и атомарной публикацией; run/control.rs — control state/server/wire protocol и публичными дочерними командами; run/executor.rs — выполнением одного Agent или Process attempt; run/scheduler.rs — frontier, публикацией готовых attempts и supervision batch. Корневой run/mod.rs переэкспортирует прежний API и содержит lifecycle start/resume. Использовать pub(super) для внутренних связей, не создавать слои или traits без новой границы.

## Acceptance Criteria

run/mod.rs не содержит inspection DTO/rendering, Unix control wire/server, Agent/Process execution, scheduler/frontier и реализации lock/read/atomic publish; публичные пути crate::run::* и crate root не меняются; durable schema и CLI output не меняются; каждый новый модуль содержит //! boundary doc; run/mod.rs существенно меньше 2000 строк; cargo fmt, ./scripts/test.sh, ./scripts/build.sh и git diff --check успешны.

## Notes

**2026-09-06T09:47:19Z**

Выполнено: src/run.rs преобразован в run/mod.rs (517 строк); выделены control.rs, executor.rs, inspection.rs, scheduler.rs и storage.rs с явными pub(super) зависимостями, прежний публичный API сохранён через re-export. Durable чтение, lock, consistent snapshot и атомарная публикация сосредоточены в storage. Проверки: cargo fmt --check, cargo clippy --all-targets -- -D warnings, cargo test --lib, ./scripts/test.sh, ./scripts/build.sh и git diff --check успешны.
