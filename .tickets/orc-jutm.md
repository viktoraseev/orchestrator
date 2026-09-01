---
id: orc-jutm
status: closed
deps: []
links: [orc-q3x2, orc-ltal]
created: 2026-09-01T13:47:52Z
type: feature
priority: 1
assignee: viktoraseev
tags: [workflow, catalog, cli, read-only, json]
---
# Перечислять source workflow templates

Добавить `orchestrator workflow list [--format text|json]`, чтобы пользователь мог обнаружить доступные WorkflowId до `validate` или `start`.

## Design

Read-only catalog boundary сканирует только `<state-root>/workflow/*.yaml`, принимает basename как WorkflowId через общий validated ID type, сортирует по WorkflowId и строит typed descriptors до renderer; содержимое workflow не materialize'ится и не проверяется этой командой.

## Acceptance Criteria

Отсутствующий `workflow/` каталог даёт успешный пустой результат; text печатает по одному WorkflowId на строку, JSON — deterministic array объектов с workflow и абсолютным path; невалидные имена contract-файлов дают code 3 без partial stdout, посторонние и temporary entries игнорируются; операция не читает config, не создаёт run и не меняет bytes; `SPEC.md`, `format.spec.md`, `cli.md` и source-tagged API/process acceptance обновлены; canonical checks проходят.
