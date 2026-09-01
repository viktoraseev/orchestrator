---
id: orc-ltal
status: closed
deps: [orc-jutm]
links: [orc-q3x2, orc-jutm]
created: 2026-09-01T13:47:52Z
type: feature
priority: 2
assignee: viktoraseev
tags: [prompt, catalog, cli, read-only, json]
---
# Перечислять prompt templates

Добавить `orchestrator prompt list [--format text|json]`, чтобы пользователь мог обнаружить PromptId и размер доступных Markdown templates.

## Design

Переиспользовать source catalog boundary: сканировать только `<state-root>/prompt/*.md`, валидировать PromptId из basename, проверять regular file и UTF-8, вычислять bytes из прочитанного snapshot и сортировать descriptors по PromptId.

## Acceptance Criteria

Отсутствующий `prompt/` каталог даёт успешный пустой результат; text печатает `prompt <id>: bytes=<n> path=<absolute-path>`, JSON — deterministic array typed descriptors; невалидный ID, non-regular или non-UTF-8 contract file даёт code 3 без partial stdout, посторонние и temporary entries игнорируются; bytes и path соответствуют одному validated snapshot, операция read-only; `cli.md`, `format.spec.md` и source-tagged API/process acceptance обновлены; canonical checks проходят.
