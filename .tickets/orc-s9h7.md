---
id: orc-s9h7
status: closed
deps: [orc-zxed]
links: [orc-abox, orc-zxed]
created: 2026-09-01T12:33:24Z
type: feature
priority: 1
assignee: viktoraseev
tags: [agent-type, claude, protocol, native-resume, process, mvp]
---
# Реализовать отдельный Claude AgentType adapter

Добавить AgentType adapter для type claude с собственными create/resume и human/non-human командами, stream-json protocol, native session activation и тем же lifecycle-контрактом completion через parent supervisor.

## Design

Закрепить Claude CLI и protocol contract в SPEC.md и cli.md до реализации; переиспользовать только process supervision и typed event sink, но не Codex args или parser; non-human запускает Claude print stream-json и извлекает session ID из init protocol, human напрямую наследует TTY; ORC_AGENT_COMMAND меняет executable без изменения Claude-аргументов и интерпретации.

## Acceptance Criteria

Process acceptance с fake executable подтверждает точные Claude args и environment для create и resume по durable session ID; model и reasoning преобразуются только в документированные Claude options; prompt и ORC_INPUT передаются без потери bytes и mapping; init session ID создаёт durable activation, повтор последнего ID остаётся idempotent; malformed JSON, неизвестная обязательная event shape, смена session ID внутри одного process и завершение без обязательного init дают fail-fast; completion-кандидат по-прежнему принимается только через attempt complete и финализируется после возврата Claude process; non-human stream-json не протекает в terminal, human Claude напрямую использует TTY; Rules получают source tags @spec:сущности, @spec:native-resume, @spec:control-endpoint-и-события, @spec:главный-workflow и @cli:вывод-команд; canonical checks проходят.


## Notes

**2026-09-01T12:52:19Z**

Реализован отдельный Claude adapter: собственные create/resume и human/non-human args, stream-json parser, durable init session activation и общий completion contract. Позитивные и негативные process acceptance проходят.
