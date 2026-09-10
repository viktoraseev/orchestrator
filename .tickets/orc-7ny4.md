---
id: orc-7ny4
status: closed
deps: []
links: []
created: 2026-09-10T16:30:50Z
type: bug
priority: 2
assignee: viktoraseev
---
# Отклонять неоднозначные depends-on alternatives

Две одновременно выполнимые ветви depends-on могут сворачиваться в одну ordered input group, поэтому durable attempt.input не различает выбранную ветвь. Validation должна отклонять такую структуру, сохраняя допустимыми взаимоисключающие alternatives по outputs.

## Acceptance Criteria

Workflow с одновременно выполнимыми alternatives одной input group отклоняется до создания run; alternatives по взаимоисключающим outputs остаются валидными; acceptance-сценарии, полный test и release build проходят.
