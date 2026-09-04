---
id: orc-zxed
status: closed
deps: [orc-r78n]
links: [orc-abox, orc-s9h7]
created: 2026-09-01T12:33:13Z
type: feature
priority: 1
assignee: viktoraseev
tags: [agent-type, codex, protocol, native-resume, process, mvp]
---
# Реализовать отдельный Codex AgentType adapter

Заменить общий process launcher для type codex отдельным AgentType adapter, который валидирует Agent, строит документированные команды новой и resumed Codex session, передаёт prompt, input mapping и control context, интерпретирует JSONL protocol и фиксирует native session activation.

## Design

Сначала закрепить конкретные Codex create/resume, human/non-human аргументы и protocol events в SPEC.md и cli.md; registry разрешает type в отдельную реализацию AgentType; ORC_AGENT_COMMAND заменяет только executable и сохраняет Codex-аргументы; non-human adapter читает codex exec --json, human adapter напрямую наследует TTY; thread/session event вызывает AttemptControl, а неизвестный или противоречивый protocol возвращает adapter error.

## Acceptance Criteria

Process acceptance с fake executable подтверждает точные Codex args и environment для новой session и native resume; model и reasoning передаются документированными Codex options; prompt и полный ORC_INPUT соответствуют immutable activation input; ORC_AGENT_COMMAND не выполняет PATH lookup и отклоняется до запуска, если path отсутствует или не executable; native session ID из JSONL durable активируется до возврата; malformed, duplicate contradictory и отсутствующий обязательный protocol event дают fail-fast без выдуманного session ID; non-human raw JSONL не попадает в stdout или stderr, human Codex напрямую получает TTY; Rules получают source tags @spec:сущности, @spec:native-resume, @spec:главный-workflow, @cli:корень-состояния-и-переменные-окружения и @cli:вывод-команд; canonical checks проходят.


## Notes

**2026-09-01T12:52:12Z**

Реализован отдельный Codex adapter: точные create/resume и human/non-human args, JSONL parser, durable session activation, executable validation и process acceptance. ./scripts/test.sh и ./scripts/build.sh проходят.
