---
id: orc-twfi
status: closed
deps: [orc-wr6w]
links: []
created: 2026-09-01T09:29:39Z
type: feature
priority: 1
assignee: viktoraseev
tags: [start, run, storage, cli, mvp]
---
# Создать восстанавливаемый run командой start

Реализовать orchestrator start [workflow-id] как первый lifecycle-срез: полностью проверить и materialize выбранный workflow до записи, атомарно зарезервировать RunId, durable-опубликовать неизменяемый spec.yaml и initial attempt 0, напечатать обещанный RunId и запустить первый non-human Agent attempt.

## Design

Добавить публичный lifecycle entrypoint и единую storage-границу run с kernel lock и атомарной публикацией целых файлов. Расширить AgentType от validation до управляемого запуска, чтобы API-сценарии использовали in-memory fake, а process-сценарий — абсолютный ORC_AGENT_COMMAND. В этом PR достаточно корректно обработать успешный возврат агента без completion как runtime error: attempt остаётся незавершённым и однозначно восстанавливаемым, но resume и control calls реализуются следующими PR.

## Acceptance Criteria

- Явный и default workflow проходят тот же resolver, materialization и validation, что validate; ошибка preflight возвращает 2, 3 или 4 и не создаёт run.
- Успешный preflight резервирует новый timestamp RunId без открытия существующего каталога, получает Run lock и последовательно durable-публикует run/<run-id>/spec.yaml и 0.<first-step>.attempt.yaml.
- spec.yaml содержит только workflow-id, эффективный max-parallel-agents и полностью materialized Steps без ссылок на изменяемые config, workflow и prompt files; initial attempt имеет пустые input и events.
- До запуска агента start печатает и flush-ит workflow: <workflow-id> и Run <run-id>; после разрешения run любое управляемое завершение заканчивается строкой Run <run-id> exited.
- Первый non-human attempt запускается через AgentType с materialized Agent, пустым input и prompt; сырой output процесса не попадает в stdout или stderr orchestrator.
- Возврат агента без completion завершает start с кодом 1, не меняет initial attempt и оставляет run пригодным для будущего resume; ошибка запуска агента имеет ту же durable-гарантию.
- API acceptance-сценарии наблюдают lifecycle result, durable snapshot и журнал fake AgentType; @process-сценарии покрывают CLI parsing, stable output, exit code, ORC_AGENT_COMMAND и границу Run lock.
- Rule и сценарии трассируются тегами @cli:start, @cli:вывод-команд, @format:materialized-workflow, @format:agent-attempt-record, @spec:сущности и @workflow:initial-activation-dependencies-и-frontier.
- ./scripts/test.sh и ./scripts/build.sh завершаются успешно.


## Notes

**2026-09-01T09:49:19Z**

Реализованы lifecycle start, полный preflight/materialization, timestamp RunId с collision retry, kernel Run lock, атомарные spec.yaml и initial attempt 0, reporter с flush-границей и AgentType API/process adapters. API и @process acceptance покрывают premature exit и конкурирующий resume; canonical ./scripts/test.sh проходит.
