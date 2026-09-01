---
id: orc-p5qt
status: closed
deps: [orc-797u]
links: []
created: 2026-09-01T11:47:32Z
type: feature
priority: 1
assignee: viktoraseev
tags: [workflow, cycles, frontier, resume, lifecycle, mvp]
---
# Повторять Step activations в циклическом workflow

Реализовать повторные активации Step в допустимом циклическом workflow: bootstrap первого Step запускается с пустым input, а последующие обходы создают новые Agent attempts только после появления свежих durable artifacts всех зависимостей.

## Design

Вычислять очередную активацию из durable lower bounds и версий artifacts без отдельного состояния цикла; использовать общую глобальную нумерацию attempts и существующую storage-границу; не менять исторические inputs и artifacts; остановку бесконечного цикла наблюдать через явный incomplete outcome, user shutdown или signal.

## Acceptance Criteria

API acceptance подтверждает последовательность a0 → b1 → c2 → a3, пустой input bootstrap-активации и свежие версии dependency artifacts на повторном обходе; номера attempts глобальны и монотонны; исторические input и artifact остаются неизменными; отсутствие terminal Step не объявляет run completed; resume продолжает незавершённый attempt и после его completion создаёт ровно одну следующую активацию без дубликатов; правила получают source tags @workflow:initial-activation-dependencies-и-frontier, @workflow:циклы-terminal-и-blocked-run и @spec:создание-и-восстановление-agent-attempt; canonical checks проходят.


## Notes

**2026-09-01T11:52:33Z**

Добавлены два API acceptance-сценария циклического workflow: bootstrap a0 → b1 → c2 → a3, fresh feedback artifacts, глобальная нумерация, отсутствие ложного completed и resume a3 → b4 без дубликатов с неизменной историей. Существующий frontier engine уже реализовывал контракт lower bounds; сквозные сценарии устранили пробел покрытия. ./scripts/test.sh, ./scripts/build.sh и cargo fmt --all -- --check проходят.
