---
id: orc-um2b
status: closed
deps: []
links: []
created: 2026-09-03T21:59:25Z
type: feature
priority: 1
assignee: viktoraseev
tags: [process, workflow, parameters, cli]
---
# Запуск произвольных бинарей как шагов workflow

Добавить второй тип исполнителя шага Process для запуска произвольного бинаря с параметрами workflow и ссылками на artifacts.

## Design

Workflow объявляет строковые parameters; start принимает повторяемый --param id=value; process задаёт executable, argv-массив args, опциональные cwd и stdout. В args поддерживаются полноэлементные placeholders {{param:id}}, {{path:step:input}} и {{output:input}} без shell expansion. Процесс получает ORC_INPUT, ORC_OUTPUT, ORC_RUN_ID, ORC_ATTEMPT и ORC_STEP_ID; stdout и файловые outputs публикуются атомарно вместе с завершением attempt; non-zero exit и отсутствующий output оставляют attempt незавершённым для at-least-once resume.

## Acceptance Criteria

Произвольный executable запускается как шаг workflow; параметры передаются через --param и подставляются в args; входные и выходные artifacts доступны через placeholders и ORC_*; stdout можно опубликовать как artifact; validation отклоняет несовместимые поля и невалидные placeholders; resume повторяет незавершённый process attempt; source show и workflow plan отображают Process и parameters; документация и acceptance-сценарии обновлены.


## Notes

**2026-09-03T21:59:38Z**

Реализовано:
- Добавлен исполнитель Process, независимый от Agent, с executable, args, cwd и публикацией stdout.
- Добавлены строковые workflow parameters и повторяемый CLI-флаг --param id=value.
- Поддержаны полноэлементные placeholders {{param:id}}, {{path:step:input}} и {{output:input}}; args передаются как argv без shell-разбора.
- Добавлены ORC_INPUT, ORC_OUTPUT, ORC_RUN_ID, ORC_ATTEMPT и ORC_STEP_ID, preflight executable/cwd, проверка outputs и атомарная фиксация artifacts с completed attempt.
- Незавершённый Process после non-zero exit или отсутствующего output повторяется командой resume с at-least-once семантикой; process-only run не создаёт Agent control socket.
- Обновлены SPEC.md, workflow.spec.md, format.spec.md и cli.md; добавлены features/process_execution.feature и tests/process_execution.rs (15 сценариев, 62 шага).
Проверено: ./scripts/test.sh, ./scripts/build.sh, cargo fmt --all -- --check — успешно.
