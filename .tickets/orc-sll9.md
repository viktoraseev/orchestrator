---
id: orc-sll9
status: closed
deps: []
links: []
created: 2026-09-07T22:29:53Z
type: bug
priority: 2
assignee: viktoraseev
---
# Различать attempt и message count в Agent board

TTY board печатает bare number после StepId; пользователь принимает volatile message count за durable attempt number, особенно после resume, когда count снова равен нулю.

## Acceptance Criteria

Каждая строка TTY board явно показывает durable attempt number и отдельно число сообщений; resume сохраняет attempt number, а новый volatile message count начинается с нуля.


## Notes

**2026-09-07T22:35:11Z**

TTY board теперь печатает StepId, durable attempt number, отдельно volatile messages count и последнее сообщение. Process acceptance-сценарий проверяет Agent attempt 1 для Codex и Claude. cargo test --test lifecycle, ./scripts/test.sh и ./scripts/build.sh прошли.
