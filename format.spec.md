# Форматы файлов orchestrator

Этот документ является единственным источником требований к layout состояния,
именам файлов и их содержимому. Семантика run и Agent attempts определена в
`SPEC.md`, workflow graph — в `workflow.spec.md`, а команды и наблюдаемое
поведение CLI — в `cli.md`.

## Корень состояния и layout

По умолчанию корнем состояния является `~/.orc`; `ORC_HOME` заменяет его по
правилам `cli.md`. Относительно выбранного корня используются только следующие
пути:

```text
config.yaml
workflow/<workflow-id>.yaml
prompt/<prompt-id>.md
run/<run-id>/active.lock
run/<run-id>/<workflow-id>.yaml
run/<run-id>/<n>-<step-id>-attempt.yaml
run/<run-id>/<n>-<step-id>-artifact-<input-id>
```

`config.yaml` необязателен. Workflow и prompt templates должны быть regular
files. `active.lock` является файлом для kernel lock; его содержимое не имеет
контракта. Volatile control socket, временные файлы атомарной записи и временные
файлы artifacts не входят в durable layout.

## Идентификаторы и номера

`RunId` записывается десятичным Unix timestamp создания в миллисекундах.
Порядковый номер Agent attempt `n` записывается десятичным целым, начинается с
`0` и относится к единой последовательности attempts всего run.

Все символические IDs, включая WorkflowId, StepId, InputId, AgentId,
AgentTypeId и PromptId, имеют kebab-case форму и соответствуют
`[a-z]+(?:-[a-z]+)*`. Начальный, конечный или повторяющийся дефис запрещён.
Native session ID является непрозрачным значением Agent type и этому формату не
подчиняется.

## Общие правила YAML

`config.yaml`, workflow template, materialized workflow и Agent attempt record
являются YAML mappings. Повторяющиеся mapping keys и неизвестные описанной ниже
схеме поля запрещены. Нарушение схемы или формата ID делает файл невалидным;
поведение использующей его команды определено в `cli.md`.

## `config.yaml`

Корневой mapping допускает только необязательные поля `default-workflow`,
`default-agent` и `agents`. `default-workflow` содержит WorkflowId,
`default-agent` — AgentId, а `agents` — mapping из AgentId в Agent:

```yaml
default-workflow: delivery
default-agent: codex-main
agents:
  codex-main:
    type: codex
    model: gpt-5-codex
    reasoning: high
```

Agent содержит ровно три обязательных строковых поля: `type` с AgentTypeId,
`model` и `reasoning`. Generic полей `command` и `environment` нет.
`default-agent` обязан ссылаться на запись в `agents`. Agent type обязан
существовать во встроенном registry, а допустимость `model` и `reasoning`
проверяет его реализация. Отсутствующий файл эквивалентен отсутствующим defaults
и пустому mapping `agents`.

## Workflow template

Workflow template находится в `workflow/<workflow-id>.yaml`, а его WorkflowId
совпадает с именем файла без `.yaml`. Корневой mapping содержит единственное
обязательное поле `steps` — непустую упорядоченную последовательность Steps с
уникальными StepIds.

Step является mapping со следующими полями:

- `id`: обязательный StepId;
- `agent`: необязательный AgentId; при отсутствии используется
  `default-agent`;
- `prompt`: необязательный PromptId;
- `human`: обязательный boolean;
- `depends-on`: обязательная последовательность уникальных StepIds; может быть
  пустой;
- `outputs`: обязательная последовательность уникальных InputIds; может быть
  пустой.

Других полей Step нет. `depends-on` описывает зависимости только от Steps, а не
от отдельных artifacts. Каждый InputId в `outputs` объявляет один обязательный
artifact успешного attempt этого Step.

Например:

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

Ссылочная целостность, cycles, reachability и семантика зависимостей заданы в
`workflow.spec.md`.

## Prompt template

Prompt template находится в `prompt/<prompt-id>.md` и является произвольным
UTF-8 Markdown. YAML-декодирование к нему не применяется. В тексте разрешены
ровно два вида placeholders:

- `{{path:<step-id>:<input-id>}}` заменяется абсолютным путём к выбранной версии
  artifact;
- `{{content:<step-id>:<input-id>}}` заменяется содержимым выбранной версии
  artifact без дополнительного экранирования.

Пробелы внутри placeholder запрещены. StepId обязан находиться в `depends-on`
использующего template Step, а InputId — в `outputs` указанного source Step.
Неизвестный вид placeholder, неизвестная пара или незакрытый `{{` делают template
невалидным. Если artifact для `content` не является UTF-8, prompt нельзя
сформировать и текущая команда завершается fail-fast до запуска агента.

Initial activation первого Step не выбирает inputs. Объявленный в его template
placeholder зависимости при этой единственной bootstrap-activation заменяется
пустой строкой; при последующих activations используется выбранная версия.

## Materialized workflow

До создания run source workflow, Agents и prompt templates materialize’ятся в
`run/<run-id>/<workflow-id>.yaml`: snapshot сохраняет имя source workflow.
Корневой mapping по-прежнему содержит только `steps` и сохраняет их исходный
порядок; WorkflowId берётся из имени файла и внутрь YAML не дублируется.

Каждый materialized Step содержит ровно следующие поля:

- `id`, `human`, `depends-on` и `outputs` со значениями source Step;
- `agent`: Agent mapping с полями `type`, `model` и `reasoning`, целиком
  скопированный из выбранного явным полем или default Agent;
- `prompt`: `null`, если template не указан, иначе строка с полным содержимым
  `.md` файла, включая неизменённые placeholders.

Materialization только заменяет ссылки `agent` и `prompt` их значениями и
записывает результат в YAML; другой обработки содержимого нет. Materialized
workflow не содержит AgentId, PromptId или ссылок на изменяемые config, workflow
и prompt files. После публикации файл неизменяем.

## Agent attempt record

Agent attempt record имеет имя `<n>-<step-id>-attempt.yaml`; имя связывает
record с номером `n` и StepId. Корневой mapping содержит ровно обязательные поля
`input` и `events`; оба являются sequences.

`input` содержит по одному номеру выбранного успешного source attempt для
каждого Step из `depends-on` в том же порядке. Для initial attempt с `n = 0`
`input` пуст независимо от `depends-on` первого Step. Номер и StepId текущего
attempt берутся из имени, а source StepIds, Agent, prompt, dependencies, outputs
и InputIds — из materialized workflow. Attempt record ничего из workflow не
materialize’ит и хранит только выбранные source attempt numbers и события.

Каждый элемент `events` является одним из mappings:

- session activation: `type: session-activated` и строковый `session-id`;
- завершение процесса: `type: process-finished`, `exit-code` и `signal`; ровно
  одно из последних двух полей содержит integer, второе равно `null`;
- completion: единственное поле `type: completed`.

Порядок sequence является durable-порядком событий. Две соседние
`session-activated` с равным `session-id` и больше одного `completed` запрещены.
Производные статусы `running`, `paused` и `completed` отдельными полями не
сохраняются.

Пример незавершённого attempt:

```yaml
input:
  - 4
events:
  - type: session-activated
    session-id: 019c-session
  - type: process-finished
    exit-code: 0
    signal: null
```

## Artifact

Artifact имеет имя `<n>-<step-id>-artifact-<input-id>` и является regular file
с произвольными bytes. Расширение файла не добавляется; содержимое не обязано
быть текстом. InputId обязан быть объявлен в `outputs` Step из имени.

Успешно завершённый attempt имеет ровно по одному artifact-файлу для каждого
InputId из `outputs`: отсутствующий, повторяющийся или дополнительный artifact
запрещён. Файл сопоставляется с Agent attempt record по `n` и StepId. Отдельный
`<n>-<step-id>-output.yaml` не создаётся. Правила публикации, неизменяемости и
восстановления определены в `SPEC.md`.
