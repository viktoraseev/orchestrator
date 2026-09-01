# Форматы файлов orchestrator

- Этот документ является единственным источником требований к layout состояния, именам файлов и их содержимому.
- Семантика run и Agent attempts определена в `SPEC.md`, workflow graph — в `workflow.spec.md`, а команды и наблюдаемое поведение CLI — в `cli.md`.

## Корень состояния и layout

- По умолчанию корнем состояния является `~/.orc`; `ORC_HOME` заменяет его по правилам `cli.md`.
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

- `config.yaml` необязателен. Workflow и prompt templates должны быть regular files.
- `active.lock` является файлом для kernel lock; его содержимое не имеет контракта.
- Volatile control socket, временные файлы атомарной записи и временные файлы artifacts не входят в durable layout.
- Source catalogs игнорируют entries с другими расширениями и имена, начинающиеся с `.`, но каждый не временный `workflow/*.yaml` и `prompt/*.md` считается contract file и обязан иметь валидный ID в basename и быть regular file.
- Show-команды строят выбранный путь только из предварительно проверенного symbolic ID и соответствующего фиксированного layout `workflow/<workflow-id>.yaml` либо `prompt/<prompt-id>.md`, не перечисляя соседние entries.

## Идентификаторы и номера

- `RunId` записывается десятичным Unix timestamp создания в миллисекундах.
- Порядковый номер Agent attempt `n` записывается десятичным целым, начинается с `0` и относится к единой последовательности attempts всего run.

- Все символические IDs, включая WorkflowId, StepId, InputId, AgentId, AgentTypeId и PromptId, имеют kebab-case форму и соответствуют `[a-z0-9]+(?:-[a-z0-9]+)*`. Начальный, конечный или повторяющийся дефис запрещён.
- Компоненты составных имён файлов разделяются точками; точка не может входить ни в один символический ID.
- Native session ID является непрозрачным значением Agent type и этому формату не подчиняется.

## Общие правила YAML

- `config.yaml`, workflow template, materialized workflow и Agent attempt record являются YAML mappings.
- Повторяющиеся mapping keys и неизвестные описанной ниже схеме поля запрещены.
- Нарушение схемы или формата ID делает файл невалидным; поведение использующей его команды определено в `cli.md`.

## `config.yaml`

- Корневой mapping допускает только необязательные поля `default-workflow`, `default-agent`, `max-parallel-agents` и `agents`.
- `default-workflow` содержит WorkflowId, `default-agent` — AgentId, `max-parallel-agents` — положительный integer общего лимита одновременно работающих процессов агентов, а `agents` — mapping из AgentId в Agent:
```yaml
default-workflow: delivery
default-agent: codex-main
max-parallel-agents: 5
agents:
  codex-main:
    type: codex
    model: gpt-5-codex
    reasoning: high
```

- Agent содержит ровно три обязательных строковых поля: `type` с AgentTypeId, `model` и `reasoning`.
- Generic полей `command` и `environment` нет.
- `default-agent` обязан ссылаться на запись в `agents`.
- Agent type обязан существовать во встроенном registry, а допустимость `model` и `reasoning` проверяет его реализация.
- Отсутствующий `max-parallel-agents` эквивалентен значению `5`; отсутствующий файл также эквивалентен отсутствующим workflow и Agent defaults и пустому mapping `agents`.

## Workflow template

- Workflow template находится в `workflow/<workflow-id>.yaml`, а его WorkflowId совпадает с именем файла без `.yaml`.
- Корневой mapping содержит единственное обязательное поле `steps` — непустую упорядоченную последовательность Steps с уникальными StepIds.

- Step является mapping со следующими полями:
  - `id`: обязательный StepId;
  - `agent`: необязательный AgentId; при отсутствии используется `default-agent`;
  - `prompt`: необязательный PromptId;
  - `human`: обязательный boolean;
  - `depends-on`: обязательная последовательность уникальных StepIds; может быть пустой;
  - `outputs`: обязательная последовательность уникальных InputIds; может быть пустой.

- Других полей Step нет. `depends-on` описывает зависимости только от Steps, а не от отдельных artifacts.
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
- Ссылочная целостность, cycles, reachability и семантика зависимостей заданы в `workflow.spec.md`.

## Prompt template

- Prompt template находится в `prompt/<prompt-id>.md` и является произвольным UTF-8 Markdown. YAML-декодирование к нему не применяется.
- В тексте разрешены ровно два вида placeholders:
  - `{{path:<step-id>:<input-id>}}` заменяется абсолютным путём к выбранной версии artifact;
  - `{{content:<step-id>:<input-id>}}` заменяется содержимым выбранной версии artifact без дополнительного экранирования.

- Пробелы внутри placeholder запрещены.
- StepId обязан находиться в `depends-on` использующего template Step, а InputId — в `outputs` указанного source Step.
- Неизвестный вид placeholder, неизвестная пара или незакрытый `{{` делают template невалидным.
- Если artifact для `content` не является UTF-8, prompt нельзя сформировать и текущая команда завершается fail-fast до запуска агента.
- Prompt template первого описанного Step не может содержать placeholders любого вида.
- Prompt catalog вычисляет `bytes` из полностью прочитанного UTF-8 содержимого файла, поэтому значение равно размеру template в bytes, а не числу Unicode symbols.
- Workflow show требует source YAML schema с непустым `steps`, уникальными валидными StepId, валидными необязательными AgentId и PromptId, валидными и уникальными значениями `depends-on` и `outputs`, но не требует существования referenced Steps, Agents, prompts или artifacts.
- Prompt show вычисляет `bytes` по точному UTF-8 содержимому выбранного template и сохраняет `content` вместе с наличием или отсутствием финального newline.

## Materialized workflow

- До резервирования RunId source workflow, Agents, prompt templates и эффективный `max-parallel-agents` materialize’ятся только в кандидат snapshot в памяти; после атомарного резервирования каталога run и получения Run lock команда `start` durable-публикует проверенный кандидат в YAML-файл `run/<run-id>/spec.yaml` с фиксированным именем.
- Корневой mapping содержит ровно `workflow-id` со значением WorkflowId source workflow, положительный integer `max-parallel-agents` с эффективным значением config на момент `start` и `steps` с исходным порядком Steps.

- Каждый materialized Step содержит ровно следующие поля:
  - `id`, `human`, `depends-on` и `outputs` со значениями source Step;
  - `agent`: Agent mapping с полями `type`, `model` и `reasoning`, целиком скопированный из выбранного явным полем или default Agent;
  - `prompt`: `null`, если template не указан, иначе строка с полным содержимым `.md` файла, включая неизменённые placeholders; при запуске `null` формирует пустую UTF-8 строку prompt без fallback или default.

- Materialization добавляет `workflow-id` и эффективный `max-parallel-agents`, заменяет ссылки `agent` и `prompt` их значениями и сериализует кандидат как YAML без записи в run; другой обработки содержимого нет.
- Materialized workflow не содержит AgentId, PromptId или ссылок на изменяемые config, workflow и prompt files.
- JSON workflow plan представляет тот же materialized кандидат с snake_case полями `workflow_id`, `max_parallel_agents` и `steps`; каждый Step содержит Agent object `{type,model,reasoning}`, точный `prompt` либо `null`, `human`, `depends_on` и `outputs`, но этот object не является durable-файлом.
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
