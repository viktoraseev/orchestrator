---
id: orc-4s5i
status: closed
deps: []
links: []
created: 2026-09-06T11:58:08Z
type: task
priority: 2
assignee: viktoraseev
tags: [specs, acceptance, recovery, cleanup]
---
# Перенести fail-fast recovery из SPEC.md в features

Синхронизировать устаревшую формулировку recovery с реализованной общей durable-моделью, сделать recovery.feature единственным контрактом раздела и удалить раздел из SPEC.md.

## Design

Под Rule о противоречивой модели перенести полную validation до scheduling и отсутствие побочных эффектов, под Rule о leftovers — исключённые из модели файлы. Состояние attempt сформулировать как производное validated workflow, полного record и artifacts без отдельного status; подтвердить успешным resume completed run. Снять @spec:fail-fast-восстановление.

## Acceptance Criteria

Все требования раздела находятся под Rules и имеют сценарии; устаревшая привязка recovery к Agent type удалена; раздел и его source-тег отсутствуют; ./scripts/test.sh, ./scripts/build.sh и git diff --check успешны.


## Notes

**2026-09-06T12:00:17Z**

Выполнено: требования fail-fast recovery перенесены под Rules в features/recovery.feature, устаревшая фраза о вычислении результата Agent type заменена фактическим контрактом derivation из validated materialized workflow, полного attempt record и artifacts без отдельного status. Добавлен сценарий успешного resume completed attempt; раздел и @spec:fail-fast-восстановление удалены. Внутренний cargo test --test lifecycle ожидаемо непригоден в sandbox из-за Unix socket EPERM; канонический ./scripts/test.sh вне sandbox и ./scripts/build.sh прошли, git diff --check чист.
