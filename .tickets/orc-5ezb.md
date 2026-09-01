---
id: orc-5ezb
status: closed
deps: [orc-s9h7]
links: []
created: 2026-09-01T12:33:37Z
type: feature
priority: 1
assignee: viktoraseev
tags: [session-view, tui, agent-board, codex, claude, observability, mvp]
---
# Показывать volatile Agent session view для non-human attempts

Собирать из Codex и Claude event streams volatile Agent session view с количеством сообщений и последним нормализованным сообщением и показывать agent board только в разрешённых TTY-состояниях без записи view в durable run.

## Design

Добавить typed observer между AgentType adapters и lifecycle presentation boundary; adapter публикует только нормализованные message events, supervisor владеет view по активному attempt, а CLI renderer получает snapshots без доступа к protocol JSON; view удаляется при возврате process и никогда не участвует во frontier, resume или attempt record.

## Acceptance Criteria

Codex и Claude parser acceptance одинаково увеличивает message count и сохраняет последнее сообщение одной строкой с детерминированной нормализацией whitespace; при TTY и отсутствии работающего human attempt board показывает каждый активный non-human attempt в порядке workflow; без TTY board не создаётся; во время human attempt board скрыт, а raw output параллельных non-human процессов не попадает в terminal; возврат process удаляет строку view, malformed message event обрабатывается по contract соответствующего adapter; session view отсутствует во всех durable files и после resume начинается заново; layout не snapshot-тестируется как стабильный API, но visibility, count и normalized text покрыты process acceptance; Rules получают source tags @spec:сущности, @spec:главный-workflow и @cli:вывод-команд; canonical checks проходят.


## Notes

**2026-09-01T12:52:27Z**

Добавлен typed volatile observer и TTY agent board: message count, whitespace normalization, last message, скрытие без TTY и при human attempt, удаление view при возврате. Codex/Claude pseudo-TTY acceptance и проверка отсутствия view в durable state проходят.
