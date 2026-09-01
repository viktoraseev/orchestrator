---
id: orc-t41e
status: closed
deps: [orc-ys4i]
links: []
created: 2026-09-01T10:56:15Z
type: feature
priority: 1
assignee: viktoraseev
tags: [human, tty, scheduler, lifecycle, mvp]
---
# Запускать human attempt напрямую в TTY

Добавить минимальный human execution path: lifecycle определяет наличие TTY, выбирает не более одного human attempt раньше non-human работы и передаёт процессу агента терминал команды напрямую; без TTY процесс не создаётся и применяется runtime fail-fast.

## Design

Расширить process-зависимости lifecycle типизированным TerminalMode, а AgentRunRequest — режимом запуска. Process adapter наследует stdin/stdout/stderr только для human attempt; API driver явно задаёт TTY и наблюдает выбор scheduler без настоящего терминала.

## Acceptance Criteria

Human attempt без TTY не запускает Agent и возвращает 1; с TTY human Agent получает прямой terminal mode; одновременно выбирается не более одного human attempt; human имеет приоритет по порядку Steps, оставшиеся slots занимают non-human attempts; Rule трассируется @workflow:планирование, @spec:agent-type и @cli:вывод-команд; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-01T11:14:01Z**

Реализован TerminalMode в публичной lifecycle-границе, human-priority scheduler и прямое наследование stdin/stdout/stderr process adapter. Headless human fail-fast не создаёт Agent; API и @process pseudo-TTY acceptance проходят. Проверки: ./scripts/test.sh, ./scripts/build.sh, cargo fmt --all --check.
