---
id: orc-vokb
status: closed
deps: [orc-t41e]
links: []
created: 2026-09-01T10:56:26Z
type: feature
priority: 1
assignee: viktoraseev
tags: [human, user-shutdown, resume, lifecycle, mvp]
---
# Обрабатывать /exit как user shutdown

Распознавать явный /exit работающего human Agent type, прекращать создание новой работы, дождаться уже запущенных non-human attempts и завершать lifecycle с кодом 0, оставляя human attempt незавершённым для resume.

## Design

Agent adapter возвращает типизированный outcome вместо кодирования /exit числом. Scheduler отличает user shutdown от runtime error, закрывает новые scheduling passes, но финализирует completion параллельных workers и печатает стабильные interrupted/resume строки.

## Acceptance Criteria

/exit не завершает human attempt и не запускает ready работу; уже работающий non-human completion становится durable; lifecycle возвращает 0 и печатает interrupted, resume и exited; следующий resume продолжает human attempt с тем же номером/session; Rule трассируется @cli:/exit-и-user-shutdown, @workflow:планирование и @spec:agent-attempt; canonical checks проходят.


## Notes

**2026-09-01T11:14:11Z**

Добавлен типизированный AgentExit::UserExit и отдельный SchedulerOutcome: /exit оставляет human attempt незавершённым, печатает interrupted/resume/exited и продолжает ту же session на resume. User shutdown дожидается уже работающих non-human completion, не создавая появившуюся после них activation.
