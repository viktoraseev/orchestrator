# Форматы файлов orchestrator

- Этот документ является источником требований к durable layout состояния, именам materialized run-файлов и их содержимому.
- Контракт config, source workflow и prompt templates, а также семантика run, attempts и workflow graph определены в `features/*.feature`, а команды и наблюдаемое поведение CLI — в `cli.md`.

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

- `config.yaml`, materialized workflow и Agent attempt record являются YAML mappings.
- Повторяющиеся mapping keys и неизвестные описанной ниже схеме поля запрещены.
- Нарушение схемы или формата ID делает файл невалидным; поведение использующей его команды определено в `cli.md`.

## Materialized workflow

- До резервирования RunId source workflow, Agents, prompt templates и эффективный `max-parallel-agents` materialize’ятся только в кандидат snapshot в памяти; после атомарного резервирования каталога run и получения Run lock команда `start` durable-публикует проверенный кандидат в YAML-файл `run/<run-id>/spec.yaml` с фиксированным именем.
- Корневой mapping содержит ровно `workflow-id`, `max-parallel-agents`, `parameters` и `steps`; `parameters` хранит mapping всех объявленных ParameterIds в точные UTF-8 значения без NUL, выбранные `start`, включая пустые строки.

- Каждый materialized Step содержит `id`, `human`, `depends-on`, `outputs`, `agent`, `prompt` и `process`; Agent Step хранит materialized `agent`, точный `prompt` либо `null` и `process: null`, Process Step хранит `agent: null`, `prompt: null` и Process mapping с абсолютными `executable` и `cwd`, неизменными `args` и `stdout`.

- Materialization добавляет `workflow-id`, effective parameters и `max-parallel-agents`, заменяет ссылки `agent` и `prompt` их значениями, разрешает Process executable и cwd и сериализует кандидат как YAML без записи в run; Process argv placeholders до запуска attempt сохраняются неизменными.
- Materialized workflow не содержит AgentId, PromptId или ссылок на изменяемые config, workflow и prompt files.
- JSON workflow plan представляет validated кандидат без runtime parameter values с snake_case полями `workflow_id`, `max_parallel_agents`, `parameters` и `steps`; `parameters` содержит ParameterIds, каждый Step содержит nullable `agent`, nullable `prompt`, nullable Process object `{executable,args,cwd,stdout}`, `human`, `depends_on` и `outputs`, но этот object не является durable-файлом.
- Durable-публикация `spec.yaml` выполняется атомарно после резервирования run и до публикации первого Agent attempt; после публикации файл неизменяем.

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
