---
id: orc-r78n
status: closed
deps: [orc-2nnu]
links: []
created: 2026-09-01T11:47:50Z
type: feature
priority: 1
assignee: viktoraseev
tags: [storage, recovery, fail-fast, resume, artifacts, mvp]
---
# Проверять и восстанавливать полную durable-модель run

Сделать resume fail-fast для всей durable-модели run: materialized workflow, attempts, inputs, artifacts и связанная нумерация проверяются до любых записей и запусков Agent, а документированные временные и orphan leftovers не принимаются за подтверждённое состояние.

## Design

Одна storage-граница сначала читает и валидирует полный immutable snapshot run, затем lifecycle принимает решение; проверять схему, ссылки, глобальную нумерацию, соответствие input выбранным версиям artifacts и completion records; атомарные temp-файлы и leftovers классифицировать по format-контракту и не удалять при чтении.

## Acceptance Criteria

Повреждённые workflow, attempt record, input или artifact дают exit code 3 до writes, Agent calls и control calls; повторный глобальный attempt number отклоняется, а gaps от orphan и temp leftovers не заполняются и их номера не переиспользуются; input со stale, отсутствующей или противоречивой версией source artifact отклоняется; допустимые temp-файлы и orphan leftovers игнорируются и не очищаются; resume после SIGKILL восстанавливается с последнего полного durable commit и продолжает тот же незавершённый attempt без дубликатов; API acceptance покрывает варианты модели, @process scenario покрывает наблюдаемое восстановление после SIGKILL, а crash point между temp и publish остаётся unit fault-injection test; правила получают source tags @spec:fail-fast-восстановление, @format:agent-attempt-record, @format:artifact, @cli:resume и @cli:сигналы-и-закрытие-терминала; canonical checks проходят.

## Notes

**2026-09-01T12:06:27Z**

Resume теперь открывает только существующий active.lock и проверяет spec.yaml, strict materialized dependencies, regular attempt records, events, inputs, completed artifacts и rendered content всех attempts до scheduling. Duplicate global numbers и противоречивые inputs дают код 3 без побочных эффектов. Temp/orphan artifacts остаются вне модели и не удаляются; их номера резервируются, поэтому orphan 99 приводит к attempt 100. Добавлен recovery.feature с 10 сценариями; существующие @process SIGKILL recovery и unit fault injection сохраняют внешнее и внутреннее crash-покрытие. ./scripts/test.sh, ./scripts/build.sh и cargo fmt --all -- --check проходят.
