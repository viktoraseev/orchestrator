---
id: orc-ti1g
status: closed
deps: []
links: []
created: 2026-09-08T07:11:36Z
type: bug
priority: 1
assignee: viktoraseev
---
# Сделать выбор режима spec-migration детерминированным

Убрать распознавание fix/repeat моделью по ORC_INPUT. Развести новую миграционную пачку и исправления review по отдельным Agent Steps, которые активируются графом по выбранным artifacts.

## Design

Control-flow определяется supervisor из depends-on и опубликованных outputs. Каждый Agent Step получает один безусловный prompt; Process Steps могут только детерминированно преобразовывать выбранный input в output ветви.

## Acceptance Criteria

Prompt агента не содержит условного выбора fix/repeat; новая пачка и исправление запускаются разными Steps; цикл review-fix и commit-next сохраняется; done завершает run; workflow validate, полный test и build проходят.


## Notes

**2026-09-08T07:21:44Z**

Разделены implement-new и implement-fix с безусловными prompts. route и decision детерминированно публикуют ветвь из ORC_INPUT; review получает единый handoff через review-input. Validate прошёл; изолированный smoke дошёл по fix-маршруту до attempt 6; ./scripts/test.sh и ./scripts/build.sh прошли.
