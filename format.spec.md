# Форматы файлов orchestrator

- Этот документ является источником требований к durable layout состояния, Agent attempt records и artifacts.
- Контракт config, source и materialized workflow, prompt templates, run, attempts и workflow graph определён в `features/*.feature`, а команды и наблюдаемое поведение CLI — в `cli.md`.

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

- `config.yaml` и Agent attempt record являются YAML mappings.
- Повторяющиеся mapping keys и неизвестные описанной ниже схеме поля запрещены.
- Нарушение схемы или формата ID делает файл невалидным; поведение использующей его команды определено в `cli.md`.

## Agent attempt record

- Agent attempt record имеет имя `<n>.<step-id>.attempt.yaml`; имя связывает record с номером `n` и StepId.
- Корневой mapping содержит ровно обязательные поля `input` и `events`; оба являются sequences.

- `input` содержит по одному номеру выбранного успешного source attempt для каждого Step из `depends-on` в том же порядке.
- Для initial attempt с `n = 0` `input` пуст независимо от `depends-on` первого Step.
- Номер и StepId текущего attempt берутся из имени, а source StepIds, Agent, prompt, dependencies, outputs и InputIds — из materialized workflow.
- Attempt record ничего из workflow не materialize’ит и хранит только выбранные source attempt numbers и события.

- Каждый элемент `events` является одним из mappings:
  - session activation: `type: session-activated` и строковый `session-id`;
  - completion: единственное поле `type: completed`.

- Порядок sequence является durable-порядком событий.
- `session-id` каждой `session-activated` обязан отличаться от последней предшествующей session activation данного attempt; `completed` может присутствовать не более одного раза и только последним событием.
- Производные статусы `running`, `paused` и `completed` отдельными полями не сохраняются.

- Пример незавершённого attempt:
```yaml
input:
  - 4
events:
  - type: session-activated
    session-id: 019c-session
```

## Artifact

- Artifact имеет имя `<n>.<step-id>.<input-id>.artifact` и является regular file с произвольными bytes.
- Суффикс `.artifact` является частью имени и не задаёт формат содержимого; файл содержит произвольные bytes и не обязан быть текстовым.
- InputId обязан быть объявлен в `outputs` Step из имени.

- Attempt с терминальным событием `completed` имеет ровно по одному artifact-файлу для каждого InputId из `outputs`: отсутствующий, повторяющийся или дополнительный artifact запрещён.
- Файл сопоставляется с Agent attempt record по `n` и StepId.
- Отдельный `<n>.<step-id>.output.yaml` не создаётся.
- Правила публикации и неизменяемости определены в `features/artifact_completion.feature`, а восстановления — в `features/recovery.feature`.
