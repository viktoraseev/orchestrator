---
id: orc-6euy
status: closed
deps: []
links: []
created: 2026-09-06T12:34:54Z
type: task
priority: 2
assignee: viktoraseev
---
# Перенести Control endpoint и события из SPEC.md в acceptance features

Перенести наблюдаемый контракт control endpoint, session activate и attempt complete в тематические Cucumber Rules, добавить недостающие сценарии, затем удалить раздел и @spec:control-endpoint-и-события.

## Acceptance Criteria

Нормативные правила раздела Control endpoint и события закреплены исполняемыми сценариями; раздел удалён из SPEC.md; source-тег отсутствует; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-06T12:35:01Z**

Начат аудит полного раздела Control endpoint и события. Цель: отдельные acceptance Rules для lifecycle endpoint, доверенного context, active-call ordering, idempotency и stale endpoint; внутреннюю механику listener не переносить как продуктовый текст.

**2026-09-06T12:49:34Z**

Перенесён весь наблюдаемый контракт раздела Control endpoint и события в features/control_endpoint.feature: 3 Rule и 7 сценариев для inherited context, отклонения чужого и закрытого context, replacement completion-кандидата, durable activation ordering, общего socket, прав 600 и SIGKILL/resume. Раздел и все @spec:control-endpoint-и-события удалены. При сверке найдено расхождение реализации: stale socket не удалялся; ControlServer теперь очищает endpoint своего run перед созданием нового, что подтверждает process-сценарий. Проверки: ./scripts/test.sh — PASS; ./scripts/build.sh — PASS; git diff --check — PASS.
