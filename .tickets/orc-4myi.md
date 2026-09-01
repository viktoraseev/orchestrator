---
id: orc-4myi
status: closed
deps: [orc-9tas]
links: []
created: 2026-09-01T10:08:26Z
type: feature
priority: 1
assignee: viktoraseev
tags: [scheduler, frontier, fanout, fanin, workflow, mvp]
---
# Вычислять fan-out и fan-in frontier детерминированно

Добавить полный ациклический frontier поверх последовательного executor: completion одного Step открывает все готовые ветви fan-out, target с несколькими dependencies создаётся только после свежей полной dependency group, а новые attempts получают глобальные номера в порядке materialized Steps.

## Design

Graph engine каждый scheduling pass выводит frontier только из materialized workflow и полной durable-модели, не сохраняет отдельные статусы и не пересчитывает статическую reachability. Пока execution остаётся однопроцессным: ready activations запускаются последовательно, что делает PR независимым от concurrency. Для каждого target хранить нижнюю границу из input предыдущей activation и выбирать максимальный доступный source attempt каждой dependency.

## Acceptance Criteria

- Completion source Step открывает fan-out во все зависящие ready Steps без условных transitions и выбора ветви по содержимому artifacts.
- На одном pass ready activations сортируются по порядку materialized Steps и получают последовательные глобальные attempt numbers в этом порядке.
- Fan-in Step создаётся только когда для каждой dependency существует свежий completed source attempt; input записывает выбранные номера в порядке depends-on.
- Для каждой dependency выбирается completed source attempt с максимальным номером меньше создаваемого attempt; промежуточные версии остаются историей.
- Один Step присутствует во frontier не более одного раза и имеет не более одного незавершённого attempt; новые source versions не меняют уже опубликованный input.
- Step с пустыми outputs остаётся полноценной dependency version и может открыть target без artifacts.
- Run completed только при отсутствии незавершённых attempts, ready activations и частично удовлетворённых dependency groups.
- Частично удовлетворённый fan-in без запускаемой работы является blocked: lifecycle возвращает 1 и перечисляет отсутствующие source Steps в детерминированном порядке workflow.
- Resume восстанавливает fan-out/fan-in frontier только из durable facts и не создаёт дубликаты attempts после повторного запуска команды.
- API acceptance-сценарии покрывают diamond graph, общий InputId у разных sources, максимальные source versions, blocked run и recovery; process используется только для стабильной blocked diagnostics.
- Rule и сценарии трассируются тегами @workflow:модель-graph, @workflow:initial-activation-dependencies-и-frontier, @workflow:циклы-terminal-и-blocked-run, @workflow:планирование и @cli:resume.
- ./scripts/test.sh и ./scripts/build.sh завершаются успешно.


## Notes

**2026-09-01T10:36:53Z**

Реализован детерминированный graph frontier: fan-out/fan-in, глобальная нумерация по materialized Step order, свежие dependency groups, versioned durable inputs и distinct keys при одинаковом InputId. Diamond и shared-output acceptance проходят.
