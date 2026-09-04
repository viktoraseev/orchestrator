---
id: orc-jygw
status: closed
deps: []
links: []
created: 2026-09-04T09:21:44Z
type: task
priority: 2
assignee: viktoraseev
tags: [example, workflow, process]
---
# Добавить демонстрационный Process workflow binary-test

Добавить запускаемый source workflow binary-test в изолированном example state root. Workflow образует простое diamond-дерево из системных binaries: parameter передаётся seed-процессу, artifacts расходятся в две ветви и объединяются финальным Process Step. План: добавить YAML, проверить source show/plan/validate, выполнить start на временной копии и затем канонические test/build.

## Acceptance Criteria

examples/binary-test/workflow/binary-test.yaml содержит parameter value и дерево seed -> left/right -> join; все Steps используют обычные binaries и Process placeholders без shell; workflow validate, plan и реальный start успешны; финальный artifact объединяет обе ветви; ./scripts/test.sh, ./scripts/build.sh и git diff --check успешны.


## Notes

**2026-09-04T09:21:53Z**

Выбран минимальный diamond graph: seed (/bin/echo) получает {{param:value}} и публикует value через stdout; left и right (/bin/cp) получают {{path:seed:value}} и {{output:*}}; join (/usr/bin/paste) получает оба path и публикует result через stdout. Shell и custom executables не используются.

**2026-09-04T09:23:57Z**

Завершено: добавлен examples/binary-test как изолированный state root с diamond workflow seed -> left/right -> join. Используются только /bin/echo, /bin/cp и /usr/bin/paste; parameter value передаётся через {{param:value}}, downstream paths через {{path:*}} и {{output:*}}. Добавлен ignore для generated run/. Проверки успешны: workflow show; workflow plan; validate; реальный start на временной копии с value=hello binary tree; финальный artifact attempt 3 result равен 'hello binary tree|hello binary tree'; ./scripts/test.sh; ./scripts/build.sh; git diff --check.
