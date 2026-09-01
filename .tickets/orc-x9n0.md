---
id: orc-x9n0
status: open
deps: []
links: []
created: 2026-09-01T13:56:11Z
type: feature
priority: 2
assignee: viktoraseev
tags: [prompt, catalog, show, cli, read-only, json]
---
# Показывать содержимое prompt template

Добавить orchestrator prompt show <prompt-id> [--format text|json], чтобы безопасно читать выбранный UTF-8 Markdown template относительно state root.

## Design

Команда парсит PromptId на CLI boundary и через prompt catalog boundary читает ровно prompt/<id>.md как regular UTF-8 file; text renderer возвращает точное содержимое без добавления newline, JSON descriptor содержит prompt, bytes, absolute path и content.

## Acceptance Criteria

Text stdout побайтово равен UTF-8 template, включая наличие или отсутствие финального newline; JSON возвращает один object с числовым bytes и точным content; неизвестный prompt возвращает 4, невалидный ID/format — 2, non-regular или non-UTF-8 — 3 без partial stdout; посторонние templates не читаются и не могут маскировать выбранный valid prompt, операция read-only; cli.md, format.spec.md и source-tagged API/process acceptance обновлены; canonical checks проходят.

