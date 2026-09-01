---
id: orc-2nnu
status: closed
deps: [orc-p5qt]
links: []
created: 2026-09-01T11:47:41Z
type: feature
priority: 1
assignee: viktoraseev
tags: [workflow, blocked, diagnostics, frontier, resume, mvp]
---
# Диагностировать blocked run по частичной dependency group

Возвращать детерминированный blocked result, когда незавершённый run не имеет runnable attempts, а target Step получил свежие artifacts только от части обязательной dependency group.

## Design

Вычислять свежесть каждой dependency относительно durable lower bounds target Step; считать blocked только группу, где есть хотя бы один свежий source и отсутствует хотя бы один обязательный source; формировать диагностику в порядке Step workflow и depends-on без введения нового persisted status.

## Acceptance Criteria

API acceptance отличает частично свежую dependency group от полностью несвежей и полностью готовой; blocked result перечисляет только отсутствующие source Steps в стабильном порядке без дублей; при blocked lifecycle не создаёт attempts, не запускает Agent и не изменяет durable-состояние; повторный resume при том же состоянии возвращает тот же результат; после durable появления недостающих artifacts resume создаёт ровно один target attempt с полным input; CLI acceptance подтверждает exit code 1 и стабильный stderr; правила получают source tags @workflow:циклы-terminal-и-blocked-run, @workflow:initial-activation-dependencies-и-frontier, @cli:resume и @cli:коды-завершения; canonical checks проходят.


## Notes

**2026-09-01T11:59:54Z**

Добавлено API- и process-покрытие blocked run: два idempotent resume возвращают код 1 и стабильный порядок missing Steps c, b без Agent calls и durable writes; CLI stderr закреплён; полная dependency group создаёт ровно один join attempt с input 1, 2. Существующий frontier уже реализовывал вычисление partial freshness и детерминированную дедупликацию. cargo test --test lifecycle --all-features проходит.
