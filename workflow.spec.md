# Workflow graph: спецификация

Этот документ определяет семантику workflow graph, его validation и правила,
по которым graph engine создаёт activations. Формат workflow-файла и описание
его полей определены в `format.spec.md`, Run storage, Agent attempts и управление
процессами — в `SPEC.md`, а публичное поведение CLI — в `cli.md`.

## Модель graph

- **Workflow** — ориентированный граф Steps. До создания run source workflow,
  prompt templates и Agents проверяются и materialize’ятся. Изменение source
  files после этого не меняет run. Порядок Steps сохраняется и является частью
  scheduling.
- **Step** — узел графа. `depends-on` перечисляет source Steps, успешное
  завершение которых требуется для activation. Зависимостей от отдельных
  artifacts нет.
- **Input mapping** принадлежит конкретному target attempt и использует пару
  `(source-step-id, input-id)` как ключ artifact: StepId определяет выбранный
  source Step, а InputId — один из объявленных им `outputs`. Отдельной сущности
  Input с собственным ID нет.
- **Artifact в graph** имеет ключ `(attempt-n, step-id, input-id)`. Все версии
  хранятся для истории. К inputs доступен только artifact успешно завершённого
  source attempt.

Успешный attempt публикует ровно все artifacts, объявленные в `outputs`. Поэтому
выбор source attempt однозначно определяет полный набор inputs от этого Step.
Step с пустым `outputs` также может быть dependency: его завершённый attempt
является версией зависимости, даже если не передаёт artifacts.

## Initial activation, dependencies и frontier

`start` создаёт initial activation первого описанного Step с пустым input и
номером attempt `0`. Это обычная activation; отдельной схемы entry Step нет.
`depends-on` первого Step игнорируется только для этой bootstrap-activation и
применяется ко всем его последующим activations.

Каждый StepId в `depends-on` образует ориентированное ребро от source Step в
target Step. Успешное завершение одного Step открывает fan-out во все target
Steps, которые от него зависят. Условных transitions, `when` и выбора ветви по
набору опубликованных artifacts нет.

Frontier — вычисляемое множество ready Steps. Один Step присутствует в нём не
более одного раза. После initial activation Step становится ready, когда для
каждого его dependency существует успешно завершённый source attempt новее
нижней границы этой зависимости. Нижняя граница — source attempt, выбранный
предыдущей activation этого target Step; initial activation с пустым input
границ не создаёт.

Для новой activation выбирается доступный source attempt с максимальным номером
для каждого dependency. Его номер должен быть меньше номера создаваемого
attempt. Выбранные номера фиксируются при создании attempt, и более поздние
source attempts их не меняют. Если между activations завершилось несколько
attempts одного source Step, выбирается только последний; остальные остаются
историей.

Например, attempt `review` с номером `11` не может выбрать source attempt `15`;
attempt `review` с номером `16` выбирает `15`, если он уже успешно завершён при
создании `16`. Следующая activation `review` обязана выбрать более новый attempt
того же dependency и не может повторно использовать `15`.

## Inputs и prompt

Input activation содержит по одному выбранному source attempt для каждого
StepId из `depends-on`. Все объявленные `outputs` выбранного source attempt
передаются target Step как mapping `(step-id, input-id) → artifact path`.
Именно в этом mapping пара является ключом. InputIds разных source Steps могут
совпадать, поскольку StepId устраняет неоднозначность.

Agent type получает этот mapping вместе с materialized Agent и сформированным
prompt. Placeholders Markdown template разрешаются из того же неизменяемого
mapping по правилам `format.spec.md`. Позднее появившиеся версии artifacts не
изменяют ни mapping, ни prompt уже созданного attempt.

## Циклы, terminal и blocked run

Граф может содержать циклы. Повторное достижение Step создаёт новый attempt;
предыдущие attempts и artifacts не переиспользуются и не изменяются.

В цикле `a → b → c → a`, где `a` описан первым, initial attempt `a` получает
пустой input. Завершение `a` активирует `b`, завершение `b` — `c`, а свежий
завершённый attempt `c` удовлетворяет `depends-on` и создаёт следующий attempt
`a`. Пустой bootstrap-input не участвует в следующих итерациях.

У цикла могут быть независимые входные, feedback- и выходные artifacts. Один
Step может зависеть одновременно от участника цикла и внешнего Step, а его
outputs могут передаваться Steps внутри и вне цикла. Специального общего cycle
input или output нет.

Step terminal, если ни один Step не содержит его ID в `depends-on`.
Run завершён, когда отсутствуют запущенные и незавершённые attempts, ready
activations и частично удовлетворённые dependencies. Dependency group частично
удовлетворена, если для target Step свеж хотя бы один, но не все source attempts;
тогда run blocked, а не completed.

## Планирование

В одном run могут одновременно существовать human и non-human attempts.

- На каждом scheduling pass supervisor обязан запустить все ready non-human
  activations параллельно.
- Одновременно выполняется не более одного human attempt. Если терминал свободен,
  выбирается ready human Step, раньше описанный в materialized workflow;
  остальные остаются во frontier.
- Non-human attempts работают параллельно с human attempt. Перед запуском новые
  activations одного scheduling pass сортируются по порядку Steps и получают
  глобальные attempt numbers в этом порядке.
- После durable-изменения, влияющего на frontier или незавершённые attempts,
  выполняется следующий scheduling pass. Native resume сохраняет прежний `n`.

## Validation

`validate` и preflight `start` проверяют:

- соответствие workflow и prompt templates `format.spec.md`;
- существование явного Agent каждого Step либо `default-agent`, валидность его
  type, model и reasoning и поддержку native resume Agent type;
- существование всех StepIds из `depends-on` и всех PromptIds;
- что каждый placeholder prompt ссылается на Step из `depends-on` и InputId из
  `outputs` этого source Step;
- статическую достижимость: первый Step достижим initial activation, а другой
  Step — только если все его dependencies достижимы; non-entry Step с пустым
  `depends-on` не имеет activation path и невалиден;
- допустимость cycles, включая `depends-on` первого Step для повторной
  activation; циклическая компонента без bootstrap-пути недостижима;
- допустимость outputs без consumers и отсутствия terminal Step.

Перед созданием run все выбранные Agents и prompt templates materialize’ятся в
снимок workflow. Во время выполнения статическая reachability повторно не
вычисляется.

## Fail-fast input validation

При создании и восстановлении attempt input обязан содержать ровно один source
attempt для каждого Step в порядке его `depends-on`. Каждый source
attempt обязан существовать, быть успешно завершённым, иметь меньший номер и
быть свежее нижней границы target Step. Для каждого InputId из его `outputs`
обязан существовать соответствующий artifact.

Неполная, старая или противоречивая input group является ошибкой fail-fast.
После публикации attempt выбранные source numbers неизменяемы.
