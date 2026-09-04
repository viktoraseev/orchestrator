---
id: orc-9i9d
status: closed
deps: []
links: []
created: 2026-09-04T04:47:18Z
type: chore
priority: 1
assignee: viktoraseev
tags: [documentation, gherkin, contract, migration]
---
# Перенести ещё 20 нормативных правил из Markdown в Gherkin

Третья партия устранения дублирования продуктового контракта: выбрать 20 наблюдаемых правил из SPEC.md, workflow.spec.md, format.spec.md и cli.md, явно закрепить их в acceptance-сценариях, удалить перенесённые Markdown-формулировки и проверить каноническими скриптами.

## Acceptance Criteria

Ровно 20 правил перечислены в истории тикета; каждое явно наблюдаемо в features/*.feature; соответствующие дубли удалены из Markdown; Gherkin и Rust step definitions проходят ./scripts/test.sh; ./scripts/build.sh успешен; git diff --check успешен.


## Notes

**2026-09-04T04:49:26Z**

Выбраны 20 правил для миграции из format.spec.md:\n1) config.yaml необязателен;\n2) workflow templates являются regular files;\n3) prompt templates являются regular files;\n4) source catalogs игнорируют другие расширения;\n5) source catalogs игнорируют имена с точкой в начале;\n6) каждый workflow/*.yaml contract file имеет валидный ID в basename;\n7) каждый prompt/*.md contract file имеет валидный ID в basename;\n8) каждый перечисляемый contract template является regular file;\n9) show проверяет symbolic ID до filesystem lookup;\n10) show строит фиксированный путь выбранного ID и не перечисляет соседей;\n11) RunId — десятичный Unix timestamp создания в миллисекундах;\n12) attempt n — десятичная глобальная последовательность run с 0;\n13) symbolic IDs соответствуют kebab-case [a-z0-9]+(?:-[a-z0-9]+)* без краевых/повторных дефисов;\n14) точка разделяет компоненты составных имён и запрещена внутри symbolic ID;\n15) native session ID непрозрачен и не подчиняется symbolic ID;\n16) корень config допускает только четыре необязательных поля;\n17) default-workflow/default-agent содержат соответствующие symbolic IDs;\n18) max-parallel-agents — положительный общий лимит, agents — mapping;\n19) Agent содержит только обязательные строковые type/model/reasoning, без command/environment;\n20) type должен быть в registry, а model/reasoning валидирует реализация Agent type.\nПосле явного покрытия удаляются соответствующие формулировки строк 21, 24-25, 29-34, 44-45 и 57-59 текущего format.spec.md; неперенесённые соседние правила остаются.

**2026-09-04T04:52:08Z**

Первый targeted run выявил code/spec mismatch: serde_yaml принимал numeric YAML scalar для String, поэтому default-workflow: 7 и Agent model: 7 завершались успешно. Для сохранения переносимого контракта обязательных строковых полей исправляется config deserialization на строгую проверку YAML string; остальные 39 новых/затронутых config scenarios прошли.

**2026-09-04T04:55:04Z**

Gherkin и targeted tests готовы: config 41/41, workflow validation 38/38, lifecycle suite и source catalog прошли. Удалены перенесённые clauses из format.spec.md, SPEC.md, workflow.spec.md и cli.md; сохранены соседние правила о layout, YAML, collision handling, artifact lookup и renderer ordering.

**2026-09-04T04:56:29Z**

Завершено: 20 правил закреплены в features/config.feature, features/source_catalog.feature, features/workflow_validation.feature и features/lifecycle.feature; дубли удалены/сокращены в format.spec.md, SPEC.md, workflow.spec.md и cli.md. Дополнительно устранён найденный mismatch: config string fields больше не принимают numeric YAML scalars. Проверки: targeted acceptance green; ./scripts/test.sh exit 0; ./scripts/build.sh exit 0; git diff --check exit 0; residual search по удалённым формулировкам пуст.
