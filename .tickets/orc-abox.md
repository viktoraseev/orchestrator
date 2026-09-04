---
id: orc-abox
status: closed
deps: []
links: [orc-s9h7, orc-zxed]
created: 2026-09-04T19:02:46Z
type: task
priority: 2
assignee: viktoraseev
tags: [rust, refactoring, agent-type]
---
# Разделить Codex и Claude Agent type adapters по модулям

Вынести type-specific реализацию встроенных Codex и Claude adapters из общего src/agent.rs в отдельные дочерние модули. Сохранить в agent.rs registry, общий process lifecycle, общий protocol event и контроль session semantics. Публичный API и продуктовое поведение не менять.

## Acceptance Criteria

Codex argv/protocol/serde-типы находятся в src/agent/codex.rs, Claude — в src/agent/claude.rs; общая process orchestration не дублируется; видимость ограничена parent module; существующие Agent type acceptance-сценарии проходят без изменений; ./scripts/test.sh, ./scripts/build.sh и git diff --check успешны.


## Notes

**2026-09-04T19:02:59Z**

Граница разделения: parent agent.rs сохраняет ProcessAgentType, AgentProtocolEvent, process_agent_type, consume_protocol и общий запуск/завершение процессов; child modules владеют default executable, type-specific argv и serde-моделью своего wire protocol. Child entrypoint имеет только pub(super) visibility.

**2026-09-04T19:06:12Z**

Завершено: общий src/agent.rs оставляет registry, ProcessAgentType, AgentProtocolEvent, process lifecycle и session semantics; Codex argv/JSONL types вынесены в src/agent/codex.rs, Claude argv/stream-json types — в src/agent/claude.rs. Дочерние модули отдают parent только pub(super) agent_type(), публичный API и feature-контракт не изменены. agent.rs сокращён с 692 до 535 строк. Проверки успешны: cargo test --lib; ./scripts/test.sh; ./scripts/build.sh; git diff --check.
