# CLI-контракт orchestrator

Этот документ задаёт публичное и agent-facing поведение CLI: команды, поиск
конфигурации, вывод, коды завершения и обработку прерываний. Модель run,
attempts, artifacts и workflow определена в `SPEC.md`.

## Пути и аргументы

- `orchestrator start [<workflow-id>]` создаёт новый run выбранного workflow.
- `orchestrator resume <run-id>` продолжает только явно указанный run. Выбор
  последнего run и другие формы implicit resume не поддерживаются.
- `orchestrator validate [<workflow-id>]` проверяет workflow без создания или
  изменения run.
- `orchestrator config get <key>` читает одно значение конфигурации.
- `orchestrator config set <key> <value>` атомарно изменяет одно значение.
- `orchestrator config list` печатает все поддерживаемые значения конфигурации.
- `orchestrator attempt complete [--artifact <artifact-id> <temporary-path>]...`
  завершает текущий attempt и одновременно публикует его artifacts.
- `orchestrator session activate <session-id>` фиксирует activation внутренней
  сессии текущего attempt.
- Необязательная общая конфигурация читается из `~/.orc/config.yaml`.
- Workflow загружается только из `~/.orc/workflow/<workflow-id>.yaml`.
- Prompt template по ссылке `<prompt-id>` загружается только из
  `~/.orc/prompt/<prompt-id>.yaml`.
- CLI не ищет workflow или prompt templates в текущем каталоге и не применяет
  fallback-пути.
- Runs находятся только в `~/.orc/run/<run-id>/`.

Символические IDs и их правило kebab-case определены в `workflow.spec.md`.
RunId является числовым исключением и состоит только из десятичных цифр. Native
session ID является непрозрачным значением Agent type и этому правилу не
подчиняется.

## Корень состояния и переменные окружения

- Пути выше отсчитываются от корня состояния. По умолчанию корень — `~/.orc`,
  где `~` разрешается через `HOME`.
- `ORC_HOME` заменяет корень целиком: при непустом значении используются
  `<ORC_HOME>/config.yaml`, `<ORC_HOME>/workflow/`, `<ORC_HOME>/prompt/` и
  `<ORC_HOME>/run/`, а `~/.orc` не читается и не изменяется. Значение обязано
  быть абсолютным путём, иначе команда завершается кодом `3` без чтения и
  изменения состояния.
- Замена корня не добавляет альтернативных мест поиска: правило об отсутствии
  поиска в текущем каталоге и fallback-путей действует и для `ORC_HOME`.
- `ORC_AGENT_COMMAND` задаёт исполняемый файл сразу для всех Agent type. При
  непустом значении запускается именно он и поиск по `PATH` не выполняется.
  Значение обязано быть абсолютным путём существующего исполняемого файла, иначе
  команда завершается кодом `3` до запуска агента и без изменения run.
- Замена исполняемого файла не меняет остальное поведение type: аргументы,
  environment запуска и интерпретация протокола остаются его собственными,
  поэтому запущенный файл различает вызвавший его type по аргументам.
- Supervisor передаёт эти переменные дочернему `orchestrator` без изменений,
  поэтому `attempt complete` и `session activate` работают в том же корне и с
  тем же выбором исполняемых файлов. Дочерний процесс их не переопределяет.

## Выбор workflow

`start` и `validate` используют один resolver:

1. Явно переданный WorkflowId всегда выбирается первым; `default-workflow` и
   список файлов для выбора не используются. Config всё равно читается позднее
   для разрешения Agents и `default-agent`.
2. Без аргумента используется `default-workflow` из `~/.orc/config.yaml`, если
   поле задано.
3. Без default выбирается единственный regular-файл с валидным WorkflowId среди
   `~/.orc/workflow/*.yaml`.
4. Если подходящих файлов нет, команда завершается с `4`.
5. Если подходящих файлов несколько, команда перечисляет их IDs в лексическом
   порядке, просит передать WorkflowId или настроить default и завершается с `2`.

`config.yaml`, если существует, является YAML mapping с необязательными полями
`default-workflow`, `default-agent` и `agents`. `agents` — mapping AgentId на
config с обязательными `type`, `model` и `reasoning`. Например:

```yaml
agents:
  codex-main:
    type: codex
    model: gpt-5-codex
    reasoning: high
default-agent: codex-main
```

Все IDs обязаны быть валидными. Agent type должен существовать во встроенном
registry, а его реализация должна принять model и reasoning Agent. Неизвестные
поля, повторяющиеся keys, неполная Agent config, неизвестный type или
несовместимые model/reasoning делают config невалидным и завершают команду с `3`.
`default-agent`, если задан, обязан ссылаться на существующий Agent. Если
`default-workflow` ссылается на отсутствующий workflow, команда завершается с
`4` и не переключается на единственный или другой доступный workflow.

## `config get`, `config set` и `config list`

Поддерживаются ключи `default-workflow` и `default-agent`. Именованные Agents
редактируются непосредственно в `config.yaml`; `config set` не создаёт и не
изменяет их конфигурацию.

- `orchestrator config get default-workflow` печатает WorkflowId без имени ключа
  или `null`, если default не задан.
- `orchestrator config get default-agent` печатает AgentId без имени ключа или
  `null`, если default не задан.
- `orchestrator config list` печатает в фиксированном порядке
  `default-workflow: <workflow-id|null>`, затем
  `default-agent: <agent-id|null>`. Команда не перечисляет workflow-файлы или
  именованные Agents.
- `orchestrator config set default-workflow <workflow-id>` проверяет формат ID и
  существование соответствующего regular-файла в `~/.orc/workflow/`, затем
  атомарно создаёт или заменяет `~/.orc/config.yaml`. Успех печатает
  `default-workflow: <workflow-id>`.
- `orchestrator config set default-agent <agent-id>` проверяет формат ID,
  валидность всего config и существование named Agent, затем атомарно заменяет
  `~/.orc/config.yaml`. Успех печатает `default-agent: <agent-id>`.

Отсутствующий `config.yaml` эквивалентен unset defaults и пустому набору Agents:
`get` и `list` завершаются с `0`. Неизвестный ключ или неверное число аргументов
завершается с `2`. Невалидный существующий config завершается с `3` и не
перезаписывается. Отсутствующий workflow для `set default-workflow` завершается
с `4`, а отсутствующий Agent для `set default-agent` — с `3`; обе ошибки не
меняют config. Config commands не получают run locks и не изменяют существующие
runs; конкурентные успешные `set` публикуют целый файл атомарно, и остаётся
значение последнего durable commit.

## Agent-facing команды

`attempt complete` и `session activate` предназначены для tool calls агента и
agent-specific hooks. Они получают RunId, номер attempt и адрес parent
supervisor только из control context текущего процесса; передать или выбрать их
аргументами нельзя. При отсутствующем или устаревшем endpoint команда
завершается с `5` и не пишет в каталог run напрямую.

`orchestrator attempt complete` принимает ноль или больше повторяющихся
`--artifact <artifact-id> <temporary-path>`. Каждый ID должен быть уникален в
запросе и объявлен в outputs текущего step, а путь должен указывать на отдельный
regular-файл во временном каталоге. Parent сначала проверяет и полностью читает
все файлы, затем одной операцией публикует весь переданный набор и completion
attempt. Без успешного completion artifacts из запроса не входят в
durable-модель run. Варианта команды, публикующего черновик без completion, нет.

`orchestrator session activate <session-id>` имеет одинаковый контракт для
создания, native resume и fork внутренней сессии: передаётся только ID
активированной сессии. Для fork это ID дочерней сессии; вид операции и parent
session ID не сохраняются и не влияют на выбор сессии для следующего resume.
Повтор ID, равного последней durable activation attempt, является успешным no-op;
тот же ID после другой activation добавляется в durable-порядок заново.

## Вывод команд

Lifecycle-сообщения имеют стабильную однострочную форму; live-вывод агентов и
Agent session views не являются машинным API.

- `start` печатает `workflow: <workflow-id>`, затем `run-id: <run-id>` в stdout и
  немедленно flush'ит обе строки до запуска первого процесса агента. К этому
  моменту materialized workflow, начальный attempt `0` и остальные обязательные
  данные run уже опубликованы durable.
- `validate` при успехе печатает `workflow <workflow-id>: valid` в stdout.
- Нормальное завершение печатает `run <run-id>: completed` в stdout.
- Явный пользовательский `/exit`, распознанный Agent type, печатает
  `run <run-id>: interrupted by user` и
  `resume: orchestrator resume <run-id>` в stdout. Attempt без completion
  остаётся незавершённым; команда завершается с кодом `0`.
- `resume` завершённого run не запускает агента, печатает
  `run <run-id>: already completed` и завершается с кодом `0`.
- Диагностика ошибок печатается в stderr и начинается с `error:`. Ошибка всегда
  содержит command context и ID известного run или workflow.
- Заблокированный частичным join run печатает отсутствующие требования в
  детерминированном порядке workflow и завершается с кодом `1`.

RunId, напечатанный `start`, является границей пользовательского обещания:
после этой строки run можно продолжить из последнего полностью опубликованного
durable-состояния. Если процесс погиб до строки, новый run пользователю не
обещан; оставшийся зарезервированный каталог не переиспользуется другим `start`.

## Коды завершения

| Код | Значение |
| --- | --- |
| `0` | Команда выполнена, run уже завершён либо пользователь явно вышел через `/exit`. |
| `1` | Ошибка выполнения: agent failure, преждевременный exit без completion, blocked run, I/O failure или другая runtime error. |
| `2` | Ошибка синтаксиса CLI, неизвестная команда, ID недопустимого формата или неоднозначный выбор workflow. |
| `3` | Невалидный config, workflow, prompt template, durable run state, control event или запрос публикации artifact. |
| `4` | Указанный workflow или run не существует. |
| `5` | Run занят другим supervisor либо требуемый control endpoint недоступен. |
| `129` | Команда прервана `SIGHUP`. |
| `130` | Команда прервана `SIGINT`, включая Ctrl-C. |
| `143` | Команда прервана `SIGTERM`. |

Ненулевой exit процесса агента не пробрасывается как код CLI: текущая команда
завершается с `1`, а исходный код сохраняется в `attempt.yaml` и показывается в
диагностике. Если одновременно работают несколько агентов, первая
зафиксированная runtime error запускает fail-fast shutdown всей команды.

Запрос `attempt complete` с повторяющимся или необъявленным ArtifactId,
не-regular source либо source вне временного каталога не меняет run; дочерний
tool call завершается с `3`, и агент может исправить запрос. Отдельной сущности
`output` нет, а содержимое artifact не валидируется как YAML.

## `start`

`start` принимает не более одного WorkflowId и разрешает его по общим правилам.
До резервирования RunId команда загружает workflow и prompt templates, выполняет
полную validation и материализует workflow. Любая ошибка на этом этапе
завершается с `2`, `3` или `4` и не создаёт run.

После успешной проверки `start` атомарно резервирует свободный timestamp RunId.
Коллизия имени повторяется с новым timestamp и не является пользовательской
ошибкой. Существующий каталог никогда не открывается как новый run, независимо
от состояния его `active.lock`.

До печати RunId команда durable-публикует минимально восстанавливаемую модель и
получает `active.lock`. После печати запускается первый описанный step. Ошибка
запуска агента оставляет восстанавливаемый run и завершается с `1`.

## `resume`

`resume` требует ровно один RunId и не выбирает run автоматически.

- Неизвестный RunId завершается с `4` без создания каталога.
- Занятый `active.lock` завершается с `5` до запуска агентов и durable-изменений.
- Невалидная или противоречивая durable-модель завершается с `3` до запуска
  агентов и durable-изменений.
- Завершённый run является успешным no-op с кодом `0`.
- Незавершённый валидный run продолжает attempts и ready activations по правилам
  `SPEC.md`.
- Blocked run без запускаемой работы завершается с `1` и перечисляет частично
  удовлетворённые joins.

Временные файлы атомарной записи, artifacts без соответствующего attempt и
файловые остатки attempt без completion не делают run повреждённым и
игнорируются по правилам `SPEC.md`.

## `validate`

`validate` принимает не более одного WorkflowId, разрешает его по общим правилам
и выполняет те же проверки, что preflight команды `start`. Полный контракт
workflow graph и validation определён в `workflow.spec.md`. Ошибки выводятся в
детерминированном порядке: сначала структура файла, затем steps в порядке
workflow, затем ссылки и граф. Невалидный workflow завершается с `3`.

## Сигналы и закрытие терминала

Первый `SIGINT`, `SIGTERM` или `SIGHUP` переводит supervisor в graceful
shutdown:

1. Новые attempts больше не создаются.
2. Тот же сигнал передаётся всем supervised agent process groups.
3. Control endpoint остаётся доступен, пока агенты завершаются. Уже принятые и
   поступившие от ещё supervised процессов tool calls, включая
   `session activate` и `attempt complete`, полностью сериализуются по обычным
   правилам. Parent завершает storage operation уже принятого запроса, даже если
   его дочерний `orchestrator` завершился до получения ответа.
4. Supervisor ждёт завершения агентов и control calls не более 10 секунд.
5. После timeout или второго termination signal оставшиеся agent process groups
   получают `SIGKILL` и reaped.
6. Supervisor завершает уже принятую атомарную storage operation, фиксирует
   наблюдённые exit facts, закрывает control endpoint и освобождает
   `active.lock`.

Команда возвращает `128 + signal`: `130` для Ctrl-C/SIGINT, `143` для SIGTERM и
`129` для SIGHUP. Эскалация по timeout или повторному сигналу не меняет код
первого сигнала.

Runtime fail-fast использует тот же shutdown protocol, передавая `SIGTERM`
оставшимся агентам, но завершается с кодом `1`.

При `SIGKILL`, аварии процесса, отключении питания или kill во время системного
вызова graceful shutdown невозможен. Ядро освобождает `active.lock`, атомарно
публикуемые временные файлы игнорируются, а следующий `resume` видит только
полностью подтверждённые durable events и завершённые attempts. Вызов
`attempt complete`, не получивший успешного ответа, мог достичь commit-точки;
его точный повтор безопасен и возвращает успех, если IDs и bytes artifacts
совпадают с уже зафиксированным результатом. Resume не обещает восстановить
volatile in-memory state, временные файлы агента или неподтверждённую работу.
