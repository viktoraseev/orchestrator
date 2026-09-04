---
id: orc-fxqq
status: closed
deps: []
links: [orc-fi0b]
created: 2026-09-04T05:07:06Z
type: chore
priority: 1
assignee: viktoraseev
tags: [documentation, gherkin, contract, migration]
---
# Перенести ещё 20 нормативных правил из Markdown в Gherkin

Пятая партия устранения дублирования продуктового контракта: выбрать следующие 20 наблюдаемых правил из SPEC.md, workflow.spec.md, format.spec.md и cli.md, закрепить их в самодостаточных acceptance-сценариях и удалить только соответствующие Markdown-дубли.

## Acceptance Criteria

Ровно 20 правил перечислены в истории тикета; каждое явно наблюдаемо в features/*.feature; соответствующие дубли удалены из Markdown без потери соседних гарантий; source tags и шаги остаются целостными; ./scripts/test.sh, ./scripts/build.sh и git diff --check успешны.


## Notes

**2026-09-04T05:08:42Z**

Выбраны 20 правил для batch 5:
1) новая non-human Codex session получает точные type-specific args;
2) native resume Codex получает точные args с session ID;
3) новая human Codex session получает точные args и прямой TTY с hook activation;
4) human native resume Codex получает точные args с session ID;
5) Codex JSONL требует непротиворечивый thread.started и идемпотентен для повтора ID;
6) Codex игнорирует неизвестные валидные events;
7) Codex agent_message обновляет только volatile session view;
8) новая non-human Claude session получает точные type-specific args;
9) native resume Claude получает точные args с session ID;
10) новая human Claude session получает точные args и прямой TTY с hook activation;
11) human native resume Claude получает точные args с session ID;
12) Claude stream-json требует непротиворечивый system/init и идемпотентен для повтора ID;
13) Claude игнорирует неизвестные валидные events;
14) Claude assistant text blocks обновляют только volatile session view;
15) ORC_AGENT_COMMAND является единым absolute executable override без PATH lookup и отклоняется до изменения run;
16) executable override сохраняет type-specific args, environment и protocol без дополнительного type-ID;
17) Agent process получает полный control environment и YAML ORC_INPUT поверх type-specific arguments;
18) Process Step получает ORC_STEP_ID, ORC_RUN_ID, ORC_ATTEMPT, YAML ORC_INPUT/ORC_OUTPUT и не получает ORC_CONTROL_ENDPOINT;
19) дочерний orchestrator наследует control environment без переопределения;
20) source catalogs читают выбранный root без materialization/run mutation и валидируются целиком до renderer без partial stdout.
Удаляются только соответствующие clauses SPEC.md и cli.md; правила config validation/default executable Agent types и отдельные Workflow show/plan остаются.

**2026-09-04T05:16:56Z**

Завершено: ровно 20 правил batch 5 закреплены в features/agent_types.feature, features/process_execution.feature, features/lifecycle.feature и features/source_catalog.feature; соответствующие дубли удалены из SPEC.md и cli.md. Добавлены process-сценарии точных Codex/Claude start, native resume и human args, прямого TTY и hook activation, идемпотентного session event, неизвестных валидных events, полного Agent/Process environment, унаследованного child control context и всех невалидных форм ORC_AGENT_COMMAND. Code/spec mismatch не найден; новый Process fixture был скорректирован под фактический контракт ORC_INPUT из step-id/input-id/path. Проверки: targeted lifecycle 21/21 Agent type scenarios без skipped, process execution 17/17, source catalog 35/35; финальный ./scripts/test.sh вне sandbox — exit 0; ./scripts/build.sh — exit 0; git diff --check — exit 0; residual search удалённых формулировок пуст.
