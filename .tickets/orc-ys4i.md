---
id: orc-ys4i
status: closed
deps: [orc-4myi]
links: []
created: 2026-09-01T10:08:45Z
type: feature
priority: 1
assignee: viktoraseev
tags: [scheduler, parallelism, agent, failfast, lifecycle, mvp]
---
# Запускать non-human attempts параллельно под общим лимитом

Сделать scheduler действительно параллельным для non-human работы: на каждом pass одновременно запускать ready и unfinished attempts не выше materialized max-parallel-agents, сохраняя детерминированное создание attempts, независимые control contexts и fail-fast всей lifecycle-команды при первой runtime error.

## Design

Разделить детерминированную фазу планирования и конкурентную фазу исполнения: supervisor под одним Run lock сначала атомарно публикует выбранные новые attempts в порядке Steps, затем запускает до лимита worker-процессов AgentType. Каждый worker имеет отдельный active attempt context, но все control calls сериализуются через один endpoint и storage boundary. Завершения могут приходить в любом порядке; следующий pass строится только из durable facts.

## Acceptance Criteria

- Число одновременно работающих Agent processes одного run никогда не превышает materialized max-parallel-agents с учётом unfinished native resume.
- Ready non-human работа выбирается в порядке materialized Steps, но после запуска может завершаться в любом порядке без изменения заранее назначенных attempt numbers и inputs.
- При исчерпанном лимите ready activations и ещё не запущенные unfinished attempts ожидают свободного slot и запускаются следующим scheduling pass той же команды.
- Каждый параллельный attempt получает независимый control context; session activation и completion одного attempt не могут изменить record или candidate другого.
- Один Unix listener обслуживает краткоживущие control calls всех активных attempts, а storage commits сериализуются без удержания in-memory mutex во время Agent execution.
- Первая runtime error до user shutdown прекращает создание новой работы, посылает SIGTERM остальным supervised process groups, завершает принятые control calls и возвращает lifecycle code 1 без проброса agent exit code.
- Успешные completion-кандидаты процессов, вернувших управление во время fail-fast, финализируются по обычным durable-правилам; незапущенная и незавершённая работа остаётся для resume.
- Разные runs могут выполняться параллельно и не разделяют lock, endpoint, лимит или attempt numbering.
- API acceptance-сценарии управляют fake Agents явными событиями без таймеров и проверяют max concurrency, порядок launch, независимость contexts и fail-fast; @process-сценарий проверяет реальные process groups и kernel lifetime.
- Rule и сценарии трассируются тегами @workflow:планирование, @spec:control-endpoint-и-события, @spec:блокировка-run, @cli:коды-завершения и @cli:сигналы-и-закрытие-терминала.
- ./scripts/test.sh и ./scripts/build.sh завершаются успешно.


## Notes

**2026-09-01T10:37:05Z**

Реализованы bounded parallel non-human attempts, независимые contexts через один Unix listener, сериализованный storage commit и fail-fast cancellation. Process adapter создаёт отдельные process groups, посылает SIGTERM соседям и эскалирует до SIGKILL через 10 секунд; принятые completion-кандидаты финализируются. API и @process acceptance проходят.
