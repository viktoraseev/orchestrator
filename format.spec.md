# Форматы файлов orchestrator

- Этот документ является источником требований к durable layout состояния и общим правилам YAML.
- Контракт config, source и materialized workflow, prompt templates, run, attempts, artifacts и workflow graph определён в `features/*.feature`, а команды и наблюдаемое поведение CLI — в `cli.md`.

## Корень состояния и layout

- По умолчанию корнем состояния является `~/.orc`.
- Относительно выбранного корня используются только следующие пути:

```text
config.yaml
workflow/<workflow-id>.yaml
prompt/<prompt-id>.md
run/<run-id>/active.lock
run/<run-id>/spec.yaml
run/<run-id>/<n>.<step-id>.attempt.yaml
run/<run-id>/<n>.<step-id>.<input-id>.artifact
```

- `active.lock` является файлом для kernel lock; его содержимое не имеет контракта.
- Volatile control socket, временные файлы атомарной записи и временные файлы artifacts не входят в durable layout.

## Общие правила YAML

- `config.yaml` является YAML mapping.
- Повторяющиеся mapping keys и поля config вне его schema запрещены.
- Нарушение схемы или формата ID делает файл невалидным; поведение использующей его команды определено в `cli.md`.
