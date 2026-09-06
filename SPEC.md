# Orchestrator: целевая спецификация

## Назначение

`orchestrator` — CLI для выполнения длинных задач агентами и обычными процессами по заранее заданному
workflow. Выполнение сохраняется как run: его можно безопасно прервать,
продолжить в другом процессе и восстановить без контекста предыдущего запуска.
Публичные команды, вывод, коды завершения и обработка сигналов определены в
`cli.md`. Workflow graph, его validation и scheduling определены в
`workflow.spec.md`. Layout состояния, имена и содержимое файлов определены в
`format.spec.md`.

## Read-only inspection

- Read-only inspection загружает materialized workflow, attempts и artifacts через ту же validation границу, что `resume`.
- Storage boundary сравнивает fingerprint всех durable `spec.yaml`, `*.attempt.yaml` и `*.artifact` до и после validation, повторяет изменившийся snapshot не более четырёх раз, возвращает стабильную противоречивую модель как validation error и непрерывно меняющуюся модель как runtime error; `active.lock` и временные entries в fingerprint не входят.
- Последняя session вычисляется из полной durable-модели; производный status отдельно не записывается.
- Watch сразу публикует initial typed snapshot, затем с фиксированным интервалом 100 ms публикует только изменившиеся validated snapshots и завершается на `blocked`, `completed` или поддерживаемом termination signal.
- Verify с явно выбранным RunId проверяет только этот run.
