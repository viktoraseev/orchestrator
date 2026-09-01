---
id: orc-q3ox
status: closed
deps: [orc-twfi]
links: []
created: 2026-09-01T09:29:58Z
type: feature
priority: 1
assignee: viktoraseev
tags: [resume, session, agent, control, cli, mvp]
---
# Сохранить session activation и продолжить attempt через resume

Реализовать минимальный native-resume вертикальный срез: supervisor принимает activation активной Agent session, durable-добавляет её в текущий attempt и команда orchestrator resume <run-id> продолжает тот же attempt и тот же номер через последнюю сохранённую session ID.

## Design

Добавить volatile control endpoint на один удерживаемый Run lock и сериализовать его события со storage commit. Расширить AgentType единым контрактом create/resume: API fake сообщает управляемые session activations напрямую той же supervisor-границе, process adapter передаёт endpoint, RunId и attempt number через environment, а дочерняя команда session activate берёт доверенный контекст только из environment. Resume сначала валидирует всю durable-модель и не мутирует её до успешной проверки.

## Acceptance Criteria

- session activate <session-id> принимается только пока parent supervisor обрабатывает соответствующий attempt; дочерняя команда не пишет файлы run напрямую.
- Новая activation атомарно добавляется в events текущего attempt до успешного ответа, одинаковый с последней activation session ID является успешным no-op, а последовательность a → b → a → a сохраняется как a → b → a.
- Недоступный или устаревший endpoint, невалидный control context и вызов после возврата агента завершаются с кодом 5 без durable-изменений.
- resume требует ровно один RunId; неизвестный run возвращает 4, занятый Run lock — 5, невалидная или противоречивая durable-модель — 3 до запуска агента и записей.
- Валидный незавершённый attempt продолжается с тем же глобальным номером и последней durable session ID; при отсутствии activations AgentType создаёт новую native session, fallback с неуспешного native resume запрещён.
- Возврат агента без completion оставляет attempt незавершённым и не запускает его повторно в той же lifecycle-команде; следующий явный resume может продолжить его снова.
- После crash или нового resume старый socket path не переиспользуется, а оставшийся volatile endpoint не входит в durable-модель.
- API acceptance-сценарии управляют fake AgentType явными событиями без таймеров; @process-сценарии покрывают session activate CLI, environment control context, kernel lock и стабильные коды/вывод.
- Rule и сценарии трассируются тегами @cli:resume, @cli:session-activate-и-attempt-complete, @format:agent-attempt-record, @spec:native-resume и @spec:control-endpoint-и-события.
- ./scripts/test.sh и ./scripts/build.sh завершаются успешно.


## Notes

**2026-09-01T09:50:28Z**

Реализованы volatile Unix control endpoint, доверенный environment context, атомарная запись session-activated с dedup последней ID, resume validation/lock и native resume того же attempt 0. API fake и process Agent подтверждают session round-trip; unknown run=4, competing lock=5. Target lifecycle: 12 scenarios, 59 steps.
