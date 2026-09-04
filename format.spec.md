# Форматы файлов orchestrator

- Этот документ является единственным источником требований к layout состояния, именам файлов и их содержимому.
- Семантика run и Agent attempts определена в `SPEC.md`, workflow graph — в `workflow.spec.md`, а команды и наблюдаемое поведение CLI — в `cli.md`.

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

- `config.yaml`, workflow template, materialized workflow и Agent attempt record являются YAML mappings.
- Повторяющиеся mapping keys и неизвестные описанной ниже схеме поля запрещены.
- Нарушение схемы или формата ID делает файл невалидным; поведение использующей его команды определено в `cli.md`.

## Workflow template

- Корневой mapping содержит необязательный `parameters` и обязательный `steps`; `parameters` является mapping уникальных ParameterIds в единственное поддерживаемое значение `string`, а `steps` — непустая упорядоченная последовательность Steps с уникальными StepIds.

- Step является mapping со следующими полями:
  - `id`: обязательный StepId;
  - `agent`: необязательный AgentId; при отсутствии используется `default-agent`;
  - `prompt`: необязательный PromptId;
  - `process`: необязательный Process mapping с обязательными `executable` и `args`, необязательными `cwd` и `stdout`; `executable` и `cwd` являются строками, `args` — последовательностью строк, `stdout` — InputId из `outputs`;
  - `human`: обязательный boolean;
  - `depends-on`: обязательная последовательность уникальных StepIds; может быть пустой;
  - `outputs`: обязательная последовательность уникальных InputIds; может быть пустой.

- Других полей Step нет. Step без `process` является Agent Step и разрешает Agent через `agent` либо `default-agent`; Step с `process` является Process Step, запрещает `agent`, `prompt` и `human: true` и не требует default Agent.
- Process `executable` не содержит placeholders; абсолютный путь используется напрямую, относительный путь с `/` разрешается относительно `cwd`, а имя без `/` разрешается через `PATH` команды `start` или preflight-команды; результат обязан указывать на executable regular file.
- Process `cwd` не содержит placeholders; отсутствующий или относительный `cwd` разрешается относительно current working directory команды, а materialized значение всегда является абсолютным путём существующего directory.
- `depends-on` описывает зависимости только от Steps, а не от отдельных artifacts.
- Каждый InputId в `outputs` объявляет один обязательный artifact успешного attempt этого Step.
- Например:
```yaml
steps:
  - id: plan
    agent: codex-main
    prompt: plan
    human: true
    depends-on: []
    outputs:
      - specification
  - id: implement
    prompt: implement
    human: false
    depends-on:
      - plan
    outputs:
      - source
```

- Например Process Step и run parameter:
```yaml
parameters:
  mode: string
steps:
  - id: convert
    process:
      executable: converter
      args: ["--mode", "{{param:mode}}", "--input", "{{path:download:source}}", "--output", "{{output:result}}"]
    human: false
    depends-on: [download]
    outputs: [result]
```
- Ссылочная целостность, cycles, reachability и семантика зависимостей заданы в `workflow.spec.md`.

## Prompt template

- Prompt template является произвольным UTF-8 Markdown. YAML-декодирование к нему не применяется.
- В тексте разрешены ровно два вида placeholders:
  - `{{path:<step-id>:<input-id>}}` заменяется абсолютным путём к выбранной версии artifact;
  - `{{content:<step-id>:<input-id>}}` заменяется содержимым выбранной версии artifact без дополнительного экранирования.

- Пробелы внутри placeholder запрещены.
- StepId обязан находиться в `depends-on` использующего template Step, а InputId — в `outputs` указанного source Step.
- Неизвестный вид placeholder, неизвестная пара или незакрытый `{{` делают template невалидным.
- Если artifact для `content` не является UTF-8, prompt нельзя сформировать и текущая команда завершается fail-fast до запуска агента.
- Prompt template первого описанного Step не может содержать placeholders любого вида.
- Workflow show требует source YAML schema с валидными ParameterIds, непустым `steps`, уникальными валидными StepId, структурно валидным Agent либо Process executor, валидными и уникальными значениями `depends-on` и `outputs` и синтаксически валидными Process placeholders, но не разрешает executable и не требует существования referenced Steps, Agents, prompts, parameters или artifacts.

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
- Правила публикации, неизменяемости и восстановления определены в `SPEC.md`.
