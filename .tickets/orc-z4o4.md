---
id: orc-z4o4
status: closed
deps: [orc-jutm]
links: [orc-fw88, orc-x9n0]
created: 2026-09-01T13:56:11Z
type: feature
priority: 1
assignee: viktoraseev
tags: [workflow, catalog, show, cli, read-only, json]
---
# Показывать source workflow template

Добавить orchestrator workflow show <workflow-id> [--format text|json], чтобы после discovery можно было прочитать структуру выбранного workflow без запуска materialization или создания run.

## Design

Команда разбирает WorkflowId на CLI boundary, выбирает ровно workflow/<id>.yaml, проверяет regular file, UTF-8 и YAML schema source workflow, затем строит typed source descriptor со Steps в исходном порядке; ссылки AgentId, PromptId и depends-on показываются как source values, но config, prompts и graph reachability не проверяются.

## Acceptance Criteria

Неизвестный workflow возвращает 4, невалидный ID или format — 2, невалидный source YAML/schema — 3 без stdout; text показывает workflow header и Steps с agent, prompt, human, depends-on и outputs в source order, JSON возвращает object с workflow, absolute path и typed steps; команда не читает config/prompt, не materialize-ит defaults и ничего не изменяет; SPEC.md, format.spec.md, cli.md и source-tagged API/process acceptance обновлены; canonical checks проходят.


## Notes

**2026-09-01T14:06:33Z**

Реализованы публичный API и CLI workflow show с source-only structural validation, text/JSON renderer, source-tagged acceptance и обновлением SPEC/format/cli; ./scripts/test.sh и ./scripts/build.sh проходят.
