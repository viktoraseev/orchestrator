---
id: orc-x87y
status: closed
deps: []
links: []
created: 2026-09-08T08:49:37Z
type: task
priority: 2
assignee: viktoraseev
tags: [spec-migration, gherkin, run-inspection]
---
# Перенести контракт read-only run inspection в features

Границы пачки: все нормативные правила раздела cli.md Read-only run inspection — consistent snapshot и четыре retry, run watch, run verify, классификация ID/artifact/durable errors и запрет partial stdout. Сначала дополнить acceptance-сценарии features/run_inspection.feature и общий inspection-контракт conditional graph, затем удалить только раздел Read-only run inspection и теги @cli:read-only-run-inspection. Поведение реализации не менять без обнаруженного расхождения.

## Acceptance Criteria

Каждое удалённое правило сопоставлено с исполняемым Rule/Scenario; targeted Cucumber checks, scripts/test.sh, scripts/build.sh и git diff --check успешны; review artifact передан через attempt complete.


## Notes

**2026-09-08T09:00:19Z**

Targeted run_inspection после усиления acceptance-контракта: 42 scenarios, 185 steps, все прошли. Раздел cli.md удалён только после успешного прогона; @cli:read-only-run-inspection снят с сохраняемых Rules.

**2026-09-08T09:07:09Z**

Финал: run_inspection 44 scenarios/195 steps; conditional_graph 27/105; scripts/test.sh, scripts/build.sh и git diff --check успешны. Каждое из 4 удалённых правил сопоставлено с сохраняемыми acceptance-сценариями в review artifact.

**2026-09-08T11:13:16Z**

Повторное ревью: усилены два timing-контракта run watch — initial snapshot измеряется немедленно до первого 100 ms poll, а polling проверяется двумя последовательными изменёнными snapshots после наблюдаемой синхронизации. Targeted Text watch 2/2 (11 steps), run_inspection 47/47 (211 steps), финальные scripts/test.sh, scripts/build.sh и git diff --check успешны. Review artifact: /private/tmp/orc-x87y-review-20260908.md.

**2026-09-08T13:09:14Z**

Повторное ревью 26: polling acceptance теперь проверяет два последовательных snapshot в диапазоне 150–300 ms; consistent snapshot управляемо изменяется после fingerprint первых трёх и четырёх attempts, observer подтверждает фактические четыре попытки, стабилизацию на четвёртой и исчерпание после четвёртой. Targeted: 3 scenarios/16 steps, полный run_inspection 48/48 scenarios и 217/217 steps, два unit retry-теста; финальные scripts/test.sh, scripts/build.sh и git diff --check успешны.

**2026-09-08T15:07:42Z**

Повторное ревью 31: polling acceptance усилен до четырёх синхронизированных интервалов с суммарным окном 360–500 ms. Production 100 ms проходит; временные реализации 75 ms (~324 ms) и 149 ms (~618 ms) адресно отклонены. Targeted scenario 1/1 (5 steps), run_inspection 48/48 scenarios (217/217 steps), scripts/test.sh, scripts/build.sh и git diff --check успешны.

**2026-09-08T16:32:19Z**

Повторное ревью 36: удалён отдельный inspect_run_with_snapshot_observer; consistent snapshot проверяется через публичные entrypoints run list/show/artifacts/artifact/verify (5/5 examples). Frontier-классификация выбранной внешней части входа repeat region выражена отдельным lifecycle Scenario blocked с missing right, а watch использует ту же топологию. Targeted run_inspection 51/51 scenarios (231/231 steps), graph scenario 1/1 (5/5 steps), unit retry 2/2; scripts/test.sh, scripts/build.sh и git diff --check успешны. Review artifact: /private/tmp/orc-x87y-review-20260908-XXXXXX.md.

**2026-09-08T18:24:09Z**

Повторное ревью 41: точная граница четырёх consistent-snapshot attempts теперь подтверждается двумя Scenario Outline через обычные публичные command entrypoints run list/show/artifacts/artifact/verify: успех после изменений первых трёх fingerprint и runtime error при изменении также четвёртого. Mutation-check: лимит 3 отклонён 5/5 success-examples, лимит 5 отклонён 5/5 exhaustion-examples; 4 восстановлен. Targeted 10/10 scenarios (45/45 steps), полный run_inspection 56/56 (251/251), unit inspection 2/2, scripts/test.sh, scripts/build.sh и git diff --check успешны.
