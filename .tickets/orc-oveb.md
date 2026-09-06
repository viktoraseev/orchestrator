---
id: orc-oveb
status: closed
deps: []
links: []
created: 2026-09-06T10:03:27Z
type: task
priority: 2
assignee: viktoraseev
tags: [specs, acceptance, cleanup]
---
# Перенести Source catalogs из SPEC.md в features

Сделать feature-сценарии единственным продуктовым контрактом оставшихся требований раздела Source catalogs в SPEC.md, затем удалить раздел и его source-теги без изменения поведения.

## Design

Полный нормативный текст workflow show разместить под соответствующим Rule в features/source_catalog.feature, workflow plan — под Rule в features/workflow_tools.feature. Сохранить существующие исполняемые сценарии как подтверждение каждого правила; удалить раздел Source catalogs из SPEC.md и снять @spec:source-catalogs, оставив остальные source-теги.

## Acceptance Criteria

Оба требования явно записаны рядом с подтверждающими сценариями; раздел Source catalogs отсутствует в SPEC.md; тег @spec:source-catalogs отсутствует; feature-файлы проходят форматную проверку и полный ./scripts/test.sh; ./scripts/build.sh и git diff --check успешны.


## Notes

**2026-09-06T10:05:33Z**

Перенесены оба оставшихся требования раздела Source catalogs: контракт workflow show находится под Rule в features/source_catalog.feature, контракт workflow plan — под Rule в features/workflow_tools.feature. Раздел удалён из SPEC.md, все @spec:source-catalogs сняты; остальные source-теги сохранены. Реализация соответствует требованиям, изменений кода не потребовалось. Проверки успешны: целевые Cucumber targets (50 сценариев), ./scripts/test.sh, ./scripts/build.sh и git diff --check.
