---
id: orc-5ijj
status: closed
deps: []
links: []
created: 2026-09-07T22:07:56Z
type: bug
priority: 2
assignee: viktoraseev
---
# Передавать review fix в следующую implement-итерацию

spec-migration workflow после review:fix запускал implement только с decision:repeat, поэтому замечания терялись и Agent выбирал новую пачку.

## Acceptance Criteria

После review:fix decision публикует fix с точным содержимым замечаний; implement получает decision:fix. После commit implement получает decision:repeat. implement:done завершает workflow через decision:done.


## Notes

**2026-09-07T22:10:03Z**

Исправлен spec-migration wiring: decision публикует fix/repeat/done, implement принимает decision:fix либо decision:repeat; fix копирует review artifact байт-в-байт. validate, plan, router test, ./scripts/test.sh и ./scripts/build.sh прошли.

**2026-09-07T23:53:23Z**

Повторное тестирование выявило неоднозначность prompt: Agent искал literal decision:fix, хотя ORC_INPUT сериализует step-id и input-id отдельными YAML-полями. Durable attempts 5/6 подтвердили корректный routing и наличие decision.fix artifact.

**2026-09-07T23:59:13Z**

Prompt теперь классифицирует ORC_INPUT по паре YAML-полей step-id/input-id: fix исправляет текущую пачку, repeat и bootstrap выбирают новую, любой иной mapping fail-fast. validate, ./scripts/test.sh и ./scripts/build.sh прошли.
