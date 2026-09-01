---
id: orc-797u
status: closed
deps: [orc-vokb]
links: []
created: 2026-09-01T10:56:39Z
type: feature
priority: 1
assignee: viktoraseev
tags: [signals, shutdown, process-group, recovery, lifecycle, mvp]
---
# Завершать lifecycle по SIGINT SIGTERM и SIGHUP

Добавить signal shutdown supervisor: первый termination signal прекращает scheduling, пересылается всем Agent process groups, сохраняет control endpoint до завершения calls и возвращает документированный код 128+signal.

## Design

Вынести signal source в process-зависимость lifecycle: production использует safe signal listener, API fake подаёт явные события. Общий cancellation хранит причину остановки, process adapter пересылает исходный signal, а второй сигнал или константный deadline 10 секунд эскалирует до SIGKILL.

## Acceptance Criteria

SIGINT/SIGTERM/SIGHUP возвращают 130/143/129 и печатают exited; новые attempts после сигнала не создаются; первый сигнал доходит до всех process groups; второй сигнал и 10-секундный deadline дают SIGKILL без изменения итогового кода; принятые control calls и completion вернувшихся процессов финализируются; SIGKILL recovery видит последний durable commit; @process acceptance не использует sleep; canonical checks проходят.


## Notes

**2026-09-01T11:14:26Z**

Добавлен safe signal-hook listener и управляемый LifecycleSignals source. SIGHUP/SIGINT/SIGTERM прекращают scheduling, пересылаются Agent process groups и возвращают 129/130/143; второй signal и константный 10-секундный deadline эскалируют до SIGKILL. @process acceptance подтверждает exited, forwarding, обе эскалации и успешный resume после kill.
