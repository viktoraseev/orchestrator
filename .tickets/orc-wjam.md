---
id: orc-wjam
status: closed
deps: []
links: []
created: 2026-09-06T10:14:22Z
type: task
priority: 2
assignee: viktoraseev
tags: [specs, acceptance, process, cleanup]
---
# Перенести Process executor из SPEC.md в features

Сделать acceptance-сценарии единственным продуктовым контрактом требований раздела Process executor в SPEC.md, затем удалить раздел и его source-теги без изменения поведения.

## Design

Распределить контракт staging, process environment, streams, process group, exit и commit между Rules в process_execution.feature; добавить process-сценарий для stdin/stdout/stderr, а отдельную process group подтвердить существующим signal-сценарием. Удалить раздел Process executor из SPEC.md и снять @spec:process-executor, сохранив остальные источники.

## Acceptance Criteria

Все требования раздела явно записаны рядом с подтверждающими сценариями; stdin/stdout/stderr имеют process-проверку; раздел отсутствует в SPEC.md; тег @spec:process-executor отсутствует; реализация соответствует тексту; ./scripts/test.sh, ./scripts/build.sh и git diff --check успешны.

## Notes

**2026-09-06T10:19:33Z**

Изначально рассматривался раздел Fail-fast восстановление, но миграция остановлена: SPEC.md приписывает вычисление результата attempt Agent type, тогда как текущие storage/scheduler используют общий AttemptRecord::is_completed(); раздел оставлен для отдельного решения. Выполнена согласованная пачка Process executor: нормативный текст распределён по Rules, раздел и @spec:process-executor удалены, добавлены process-сценарии для stdin/stdout/stderr и non-regular output, а retry теперь подтверждает отсутствие публикации staging output при ненулевом exit. Проверки успешны: cargo test --test process_execution (19 сценариев), ./scripts/test.sh, ./scripts/build.sh и git diff --check.
