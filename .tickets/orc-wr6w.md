---
id: orc-wr6w
status: closed
deps: [orc-p97s]
links: []
created: 2026-09-01T09:05:03Z
type: feature
priority: 1
assignee: viktoraseev
tags: [validate, workflow, config, cli, mvp]
---
# Проверить workflow по умолчанию

Расширить orchestrator validate формой без аргумента: выбрать только default-workflow из config.yaml и передать разрешённый WorkflowId в тот же полный validate entrypoint, что использует явная форма команды.

## Design

Выделить общий workflow resolver для будущих validate и start. Resolver различает явный и неявный выбор, не сканирует workflow каталог и не применяет fallback. После разрешения обе формы используют идентичные materialization, registry и validation; отдельной реализации graph checks для default-path быть не должно.

## Acceptance Criteria

- validate без аргумента выбирает default-workflow, при успехе печатает workflow <workflow-id>: valid и завершается с кодом 0.
- Отсутствующий default-workflow завершается с кодом 2 и предлагает передать WorkflowId или настроить default.
- Ссылка default-workflow на отсутствующий template завершается с кодом 4 без выбора другого workflow.
- Невалидный config завершается с кодом 3 до чтения workflow, а невыбранные templates не читаются и не валидируются.
- Явная и default-формы после разрешения вызывают один публичный validate entrypoint и возвращают одинаковые validation errors для одного кандидата.
- Acceptance-сценарии имеют Rule с тегами @cli:выбор-workflow и @cli:validate; CLI parsing и стабильный вывод проверяются в @process-режиме, семантика resolver и validation не дублируется.
- ./scripts/test.sh и ./scripts/build.sh завершаются успешно.


## Notes

**2026-09-01T09:26:00Z**

Реализован общий resolver explicit/default без scan/fallback; обе формы используют один validate entrypoint. Добавлены API и process-сценарии для успеха и кодов 2/3/4. Проверки: ./scripts/test.sh, ./scripts/build.sh.
