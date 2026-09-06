---
id: orc-wo0d
status: closed
deps: []
links: []
created: 2026-09-06T15:35:43Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести модель graph и activation semantics в acceptance features

Перенести разделы Модель graph, Initial activation dependencies и frontier, Inputs и prompt из workflow.spec.md в исполняемые Rules соответствующих feature-файлов, затем удалить разделы и их source-теги.

## Acceptance Criteria

Все нормативные положения трёх разделов отражены в Rules с существующими или новыми acceptance-сценариями; сценарии не удалены; разделы и теги удалены; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T15:39:26Z**

Перенесены 16 нормативных положений разделов Модель graph, Initial activation dependencies и frontier, Inputs и prompt в Rules lifecycle, graph execution, process execution и artifact completion. Три раздела и source-теги удалены, обзорные ссылки в AGENTS.md, cli.md, format.spec.md и workflow.spec.md актуализированы. Ни один Scenario не удалён или изменён. Проверки: ./scripts/test.sh, ./scripts/build.sh, git diff --check.
