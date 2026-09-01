---
id: orc-jlen
status: closed
deps: [orc-q3ox]
links: []
created: 2026-09-01T09:30:17Z
type: feature
priority: 1
assignee: viktoraseev
tags: [attempt, artifact, completion, run, cli, mvp]
---
# Завершить single-step run с durable artifacts

Реализовать успешное завершение одного non-human Step: активный агент передаёт полный кандидат attempt complete, supervisor финализирует его только после возврата процесса, атомарно публикует artifacts и terminal completed, после чего single-step run становится завершённым.

## Design

Провести attempt complete через тот же control endpoint и активный attempt context. Parent полностью валидирует mapping outputs и читает source regular files до замены volatile completion candidate; повтор заменяет кандидат целиком. При возврате AgentType storage сначала публикует artifacts под durable-именами, затем одной атомарной заменой attempt record добавляет completed как commit-точку; recovery игнорирует файлы без terminal event.

## Acceptance Criteria

- attempt complete принимает ровно по одному уникальному объявленному InputId с абсолютным существующим path на regular file, а Step с пустым outputs принимает пустой список.
- Отсутствующий, повторяющийся или дополнительный InputId, относительный/несуществующий path и конечный объект не regular file возвращают 3 и не изменяют предыдущий кандидат.
- Каждый валидный повтор во время активной обработки возвращает 0 и целиком заменяет предыдущий кандидат bytes; source files читаются полностью до принятия.
- Completion candidate остаётся volatile и не завершает attempt до возврата agent process; session activations продолжают приниматься после candidate.
- После возврата процесса полный набор artifacts публикуется под именами <n>.<step-id>.<input-id>.artifact, затем terminal completed фиксируется одной атомарной заменой attempt record и делает artifacts доступными durable-модели.
- Crash/fault до commit terminal record не создаёт завершённый attempt; временные и orphan artifact files игнорируются, а resume видит последнее полностью подтверждённое состояние.
- Завершение единственного terminal Step печатает run <run-id>: completed, затем Run <run-id> exited и возвращает 0; resume завершённого run не запускает агента, печатает already completed и возвращает 0.
- Ненулевой exit агента не пробрасывается как CLI code: кандидат всё равно финализируется, после чего lifecycle fail-fast завершается с кодом 1 по runtime-контракту.
- API acceptance-сценарии проверяют candidate replacement, durable bytes, terminal commit и журнал fake AgentType; @process-сценарии покрывают attempt complete CLI и итоговый stdout/exit code.
- Rule и сценарии трассируются тегами @cli:session-activate-и-attempt-complete, @cli:вывод-команд, @format:artifact, @format:agent-attempt-record, @spec:публикация-artifacts-и-completion и @workflow:циклы-terminal-и-blocked-run.
- ./scripts/test.sh и ./scripts/build.sh завершаются успешно.


## Notes

**2026-09-01T09:53:00Z**

Реализованы attempt complete через parent control endpoint, полная validation InputId/path, replacement volatile candidate, публикация artifact bytes и terminal completed как commit-точка после возврата Agent. Покрыты 6 классов невалидных запросов с кодом 3, nonzero Agent exit после финализации, completed resume no-op и fault injection. Lifecycle: 19 scenarios/100 steps; canonical test/build и fmt check проходят.
