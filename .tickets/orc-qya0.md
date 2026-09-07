---
id: orc-qya0
status: closed
deps: []
links: []
created: 2026-09-07T06:08:00Z
type: feature
priority: 2
assignee: viktoraseev
---
# Условные outputs и зависимости, ограниченные циклы без fresh

Рекурсивные all/one-of в outputs и depends-on, qualified artifact references, повторяемый участок с одним входом и решением после всех выбранных ветвей; постоянный внешний контекст переиспользуется. Синхронизировать features, CLI, durable recovery и Process completion.


## Notes

**2026-09-07T06:47:34Z**

Реализованы рекурсивные all/one-of, qualified artifact references, точный completion snapshot, согласованная reachability, единый повторяемый участок с барьером перед решением и переиспользованием внешнего контекста. Durable input groups неизменяемы, невыбранные ветви закрываются без blocked, Process resume очищает неподтверждённые outputs. Обновлены executable features и CLI. Проверено: ./scripts/test.sh (367 acceptance-сценариев, unit tests, fmt, Clippy) и ./scripts/build.sh успешно.
