---
id: orc-fi0b
status: closed
deps: []
links: [orc-fxqq]
created: 2026-09-04T06:21:54Z
type: chore
priority: 1
assignee: viktoraseev
tags: [documentation, gherkin, contract, migration]
---
# Перенести контракты Codex и Claude Agent type в Gherkin

Шестая партия устранения дублирования продуктового контракта: перенести оставшиеся разделы Codex Agent type и Claude Agent type из SPEC.md в самодостаточные acceptance-сценарии, сохранив валидацию model/reasoning и default executable каждого встроенного type. План: 1) инвентаризировать существующее покрытие; 2) добавить Rule и явные сценарии; 3) удалить только перенесённые Markdown-разделы; 4) выполнить канонические проверки.

## Acceptance Criteria

Ровно два оставшихся нормативных правила Codex/Claude перечислены в истории тикета; каждое явно наблюдаемо в features/*.feature и помечено действующим @spec-тегом; соответствующие разделы удалены из SPEC.md без потери контракта; ./scripts/test.sh, ./scripts/build.sh и git diff --check успешны.


## Notes

**2026-09-04T06:22:03Z**

Выбраны два оставшихся правила: 1) type codex принимает непустой model и reasoning из low/medium/high/xhigh/max, executable по умолчанию codex; 2) type claude принимает непустой model и reasoning из low/medium/high/xhigh/max, executable по умолчанию claude. Тикет связан с предыдущим batch orc-fxqq.

**2026-09-04T06:26:26Z**

Завершено: два оставшихся правила Codex/Claude перенесены в features/agent_types.feature и привязаны к действующему @spec:сущности. Для каждого type явно проверены пять допустимых reasoning, пустой model, пустой/неизвестный reasoning и запуск default executable через изолированный PATH без ORC_AGENT_COMMAND. Разделы Codex Agent type и Claude Agent type удалены из SPEC.md; code/spec mismatch не найден, production implementation менять не потребовалось. Проверки успешны: cargo test --test lifecycle; ./scripts/test.sh; ./scripts/build.sh; git diff --check.
