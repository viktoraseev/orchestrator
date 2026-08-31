# Workflow graph: спецификация

Этот документ определяет YAML-модель workflow, её validation и правила,
по которым graph engine создаёт activations. Run storage, Agent attempts и
управление процессами определены в `SPEC.md`, а публичное поведение CLI — в
`cli.md`.

## Модель workflow

- **Workflow** — YAML-шаблон ориентированного графа steps. Его ID совпадает с
  именем файла без `.yaml`. До создания run шаблон загружается, проверяется и
  материализуется; ошибка на этом этапе не создаёт run. Run использует
  собственный снимок workflow, поэтому изменение исходного шаблона не меняет
  уже начатое выполнение. Порядок описания steps сохраняется при
  материализации и является частью семантики workflow.
- **Step** — узел с уникальным ID, необязательным `agent: <agent-id>`,
  необязательной ссылкой на YAML-шаблон prompt, `human: bool` и списком outputs.
  Без `agent` Step использует `default-agent` из config. `join: all` опционален;
  если он задан, это непустой список различных пар `step-id` и `artifact-id`.
  Отсутствие join не создаёт join-derived activations. Outputs задаёт разрешённые
  ArtifactIds, но не требует публиковать каждый из них. Отдельных transitions,
  `when` и routing result нет.
- **Artifact в graph** имеет ключ `(attempt-n, step-id, artifact-id)`. Все
  версии хранятся для истории. Для одной пары `(step-id, artifact-id)` graph
  выбирает версию с максимальным подходящим `attempt-n`; к требованиям join
  доступен только artifact успешно завершённого attempt.

Все символические IDs, включая WorkflowId, StepId, ArtifactId, AgentId,
AgentTypeId и PromptId, соответствуют `[a-z]+(?:-[a-z]+)*`.

## Initial activation, joins и frontier

`start` создаёт initial activation первого описанного step с пустым input, то
есть без выбранных artifact versions. Это обычная activation и Agent attempt с
номером `0`; отдельной схемы Step или attempt для начала run нет. Если у первого
step объявлен `join: all`, он применяется только к последующим activations.

Каждое требование `join: all` образует ориентированное ребро от пары `(source
step-id, artifact-id)` в target step. Публикация нескольких outputs может открыть
несколько ветвей; один output может открыть fan-out в несколько target steps.

Frontier — вычисляемое множество ready steps. Один step присутствует в нём не
более одного раза. После initial activation step получает join-derived activation
только когда для каждого требования его `join: all` доступен artifact свежее
нижней границы. Нижняя граница требования — версия, выбранная последним attempt
этого step для этого требования; initial attempt с пустым input границ не создаёт.

Для новой activation выбирается максимальная доступная версия каждого требования
с номером меньше номера создаваемого attempt. Выбранные номера фиксируются в
attempt и позднейшие artifacts их не изменяют. Если между activations появились
несколько версий, создаётся одна activation с последней версией; остальные
остаются историей.

Например, attempt `review` с номером `11` не может выбрать artifact attempt
`15`; attempt `review` с номером `16` выбирает `15`, только если он уже доступен
при создании attempt. Старые версии не могут удовлетворить следующий join в
цикле.

## Циклы, terminal и blocked run

Граф может содержать циклы. Повторное достижение step создаёт новый attempt;
прошлые attempts и artifacts не переиспользуются и не изменяются.

В цикле `a → b → c → a`, где `a` описан первым, initial attempt `a` получает
пустой input. Output `a` активирует `b`, output `b` — `c`, а свежий output `c`
удовлетворяет join и создаёт следующий attempt `a`. Пустой input не участвует в
следующих итерациях.

У цикла могут быть независимые входные, feedback- и выходные artifacts. Join
участника может дополнительно ждать artifacts уже запущенных ветвей, а его
outputs могут одновременно продолжить цикл и открыть steps вне него. Специального
общего cycle input или output нет.

Step terminal, если ни один join не ссылается на его outputs. Нетерминальный step
может штатно закончить динамическую ветвь, не опубликовав downstream artifact.
Run завершён, когда отсутствуют запущенные и незавершённые attempts, ready
activations и частично удовлетворённые joins. Join частично удовлетворён, если
свежа хотя бы одна, но не все его версии; тогда run blocked, а не completed.

## Планирование

Поле `human` относится к Step. В одном run могут одновременно существовать human
и non-human attempts.

- На каждом scheduling pass supervisor обязан запустить все ready non-human
  activations параллельно.
- Одновременно выполняется не более одного human attempt. Если терминал свободен,
  выбирается ready human step, раньше описанный в materialized workflow; остальные
  остаются во frontier.
- Non-human attempts работают параллельно с human attempt. Перед запуском новые
  activations одного scheduling pass сортируются по порядку steps и получают
  глобальные attempt numbers в этом порядке.
- После durable-изменения, влияющего на frontier или незавершённые attempts,
  выполняется следующий scheduling pass. Native resume сохраняет прежний `n`.

## Validation

`validate` и preflight `start` проверяют:

- YAML-схему workflow, отсутствие duplicate mapping keys и неизвестных полей,
  включая удалённые `transitions` и `when`;
- совпадение WorkflowId с именем файла, валидность всех IDs и непустую
  упорядоченную последовательность уникальных Steps;
- что у любого Step join отсутствует либо является непустым `join: all`, outputs
  содержат уникальные ArtifactIds, а `human` является boolean;
- что явный Agent Step существует, а при его отсутствии существует
  `default-agent`; type, model и reasoning каждого выбранного Agent валидны по
  registry `AgentType` и type поддерживает native resume; существование,
  materialization и валидность указанного prompt template;
- уникальность пар в одном join, существование source step и объявление им
  требуемого ArtifactId в outputs;
- статическую достижимость: initial activation первого Step образует начальное
  множество, а другой Step добавляется только при наличии join и достижимости
  producers всех его требований;
- допустимость циклов, включая join первого Step для повторной activation.
  Циклическая компонента без bootstrap-пути от initial activation недостижима;
- допустимость outputs без consumers и отсутствия статически terminal Step.

Перед созданием run все выбранные Agents резолвятся и materialize’ятся в снимок
workflow. Содержимое Artifact — произвольные bytes и validation workflow его не
разбирает. Во время выполнения статическая reachability повторно не вычисляется.

## Durable input activation

Initial attempt с `n = 0` хранит пустой input. Каждый subsequent attempt хранит
ровно один выбранный source `attempt-n` для каждого требования `join: all`.
Выбранный artifact обязан существовать, принадлежать успешно завершённому
attempt, иметь меньший номер, чем потребляющий attempt, и быть новее версии того
же требования в предыдущем attempt этого Step. Эти входные номера фиксированы
после публикации attempt и составляют часть fail-fast восстановления run.
