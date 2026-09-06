---
id: orc-7b94
status: closed
deps: []
links: []
created: 2026-09-06T13:34:50Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести блокировку run из SPEC.md в acceptance features

Перенести наблюдаемый контракт RunId reservation и lifetime exclusive Run lock в тематические Cucumber Rules, добавить недостающие конкурентные сценарии, затем удалить секцию и её source-теги.

## Acceptance Criteria

Все нормативные правила секции Блокировка run подтверждены acceptance-сценариями; секция удалена из SPEC.md; @spec:блокировка-run отсутствует; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T13:34:57Z**

Начат аудит полного контракта RunId reservation и Run lock против lifecycle, graph, signal и control acceptance-сценариев и storage реализации.

**2026-09-06T13:47:06Z**

Перенесён весь наблюдаемый контракт секции «Блокировка run» в features/run_lock.feature: существующий busy-lock сценарий перемещён без изменения, добавлены сценарии атомарного резервирования разных RunId конкурирующими start и удержания lock между последовательными Steps с освобождением для resume. Секция удалена из SPEC.md, теги @spec:блокировка-run удалены, Rustdoc storage ссылается на новый Rule. Расхождений контракта с реализацией не обнаружено. Проверки: cargo test --test lifecycle; ./scripts/test.sh; ./scripts/build.sh; git diff --check — успешно.

**2026-09-06T13:50:02Z**

Финальный аудит добавил четвёртый acceptance-сценарий для оставшегося нормативного случая: ошибка резервирования, не являющаяся коллизией, завершает start с кодом 1, не запускает Agent и не изменяет занявший путь regular file. После дополнения повторно успешно прошли cargo test --test lifecycle, ./scripts/test.sh, ./scripts/build.sh и git diff --check.
