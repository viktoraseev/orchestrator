---
id: orc-ar0z
status: closed
deps: []
links: []
created: 2026-09-03T22:20:16Z
type: chore
priority: 1
assignee: viktoraseev
tags: [documentation, gherkin, contract, migration]
---
# Миграция продуктовых контрактов из Markdown в Gherkin: batch 2

Перенести ещё 10 дублирующихся продуктовых правил из SPEC.md, workflow.spec.md, format.spec.md и cli.md в самодостаточные acceptance-сценарии, после чего удалить Markdown-дубли.

## Acceptance Criteria

Выбраны ровно 10 правил; ожидаемые значения и формы видны непосредственно в features; соответствующие Markdown-дубли удалены без потери неперенесённых гарантий; source tags остаются целостными; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-03T22:20:24Z**

Начата миграция batch 2. Кандидаты: workflow list, agent list, prompt list, workflow show, agent show, bulk validate, workflow plan, явный validate, default validate и ORC_HOME. Сначала проверяется полнота acceptance-сценариев; составные Markdown-строки будут сокращаться только в перенесённой части.

**2026-09-03T22:25:32Z**

Gherkin усилен для второй десятки: добавлены точные text/JSON формы workflow, Agent и Prompt catalogs, пустые catalogs, полные Agent fields, полный bulk validate report, ошибка двух WorkflowId, точный config list и выбор state root для absolute/empty/relative ORC_HOME. Соответствующие дубли удалены или сокращены в SPEC.md, workflow.spec.md, format.spec.md и cli.md; global I/O validate --all и workflow show/plan оставлены как ещё не перенесённые.

**2026-09-03T22:26:52Z**

Batch 2 завершён. Перенесены 10 правил: workflow catalog, Agent catalog, Prompt catalog, Agent show, полный bulk validate report, явный validate, default validate, отсутствие выбранного template, config list и ORC_HOME. Добавлены точные Gherkin-ожидания для text/JSON, пустых catalogs, Agent fields, Prompt bytes, двух WorkflowId и absolute/empty/relative ORC_HOME. Удалены соответствующие дубли из SPEC.md, workflow.spec.md, format.spec.md и cli.md; неперенесённые global I/O validate --all и workflow show/plan сохранены. Проверено: targeted Cucumber targets, ./scripts/test.sh вне sandbox, ./scripts/build.sh и git diff --check — успешно.
