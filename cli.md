# CLI-контракт orchestrator

Этот документ задаёт публичное и agent-facing поведение CLI: команды, поиск конфигурации, вывод, коды завершения и обработку прерываний. Модель run, attempts и artifacts определена в `features/*.feature`, workflow graph — в `workflow.spec.md`, а layout состояния и форматы файлов — в `format.spec.md`.

## Команды и аргументы

- `orchestrator start [<workflow-id>] [--param <parameter-id>=<value>]...` создаёт новый run выбранного workflow.
- `orchestrator resume <run-id>` продолжает только явно указанный run. Выбор последнего run и другие формы implicit resume не поддерживаются.
- `orchestrator run list [--state active|blocked|completed]... [--workflow <workflow-id>] [--format text|json]` читает и перечисляет все durable runs без запуска Agent или изменения состояния.
- `orchestrator run show <run-id> [--format text|json]` читает один durable run и показывает его materialized Steps, attempts и вычисленный frontier.
- `orchestrator run artifacts <run-id> [--format text|json]` перечисляет опубликованные версии durable artifacts.
- `orchestrator run artifact <run-id> <attempt-n> <input-id>` пишет в stdout точные bytes опубликованного artifact выбранного attempt.
- `orchestrator run watch <run-id> [--format text|json]` наблюдает изменения validated snapshot до terminal state или signal.
- `orchestrator run verify [<run-id>] [--format text|json]` формирует полный read-only validation report.
- `orchestrator workflow show <workflow-id> [--format text|json]` показывает структуру выбранного source workflow template.
- `orchestrator workflow plan <workflow-id> [--format text|json]` показывает полностью materialized execution plan без создания run.
- `orchestrator attempt complete [--artifact <input-id> <path>]...` передаёт полный текущий кандидат completion и artifacts; attempt завершается только после возврата процесса агента.
- `orchestrator session activate <session-id>` фиксирует activation внутренней сессии текущего attempt.
Файлы config, workflow, prompt templates и runs размещаются только по layout из `format.spec.md`. CLI не ищет их в текущем каталоге и не применяет fallback-пути.

## Корень состояния и переменные окружения

- Пути из `format.spec.md` отсчитываются от корня состояния. Корень по умолчанию задан там и разрешается через `HOME`.
- Замена корня не добавляет альтернативных мест поиска: правило об отсутствии поиска в текущем каталоге и fallback-путей действует и для `ORC_HOME`.

## Выбор workflow

После выбора workflow config всё равно читается для разрешения Agents и `default-agent`.

## `config get`, `config set` и `config list`

Именованные Agents редактируются непосредственно в config; `config set` не создаёт и не изменяет их конфигурацию.

Config commands не получают run locks и не изменяют существующие runs.

## Agent-facing команды

- `attempt complete` и `session activate` предназначены для tool calls агента и agent-specific hooks.
- Они получают RunId, номер attempt и адрес parent supervisor только из control context текущего процесса; передать или выбрать их аргументами нельзя.
- При отсутствующем или устаревшем endpoint либо после того, как parent наблюдал возврат процесса агента, обрабатывавшего attempt, команда завершается с `5` и не пишет в каталог run напрямую.

- `orchestrator attempt complete` принимает ноль или больше повторяющихся `--artifact <input-id> <path>`.
- Набор InputIds обязан в точности совпадать с `outputs` текущего Step; для пустого `outputs` аргументы `--artifact` не передаются.
- Каждый `<path>` является переданным агентом абсолютным путём к source-файлу artifact; отдельный временный каталог orchestrator не создаёт и не передаёт, containment пути не проверяется, symbolic links разрешаются операционной системой, а конечный объект обязан быть regular file.
- Parent сначала проверяет и полностью читает все файлы, затем целиком заменяет ими последний принятый кандидат текущей обработки attempt.
- Каждый следующий валидный `attempt complete`, принятый пока обработка attempt активна, может передать другие bytes artifacts и снова целиком заменяет предыдущий кандидат.
- До возврата процесса агента кандидат не завершает attempt и его artifacts не доступны workflow graph; при возврате последний принятый кандидат публикуется по Rule «Completion становится durable только после возврата Agent» в `features/artifact_completion.feature`.

- `orchestrator session activate <session-id>` имеет одинаковый контракт для создания, native resume и fork внутренней сессии: передаётся только ID активированной сессии.
- Для fork это ID дочерней сессии; вид операции и parent session ID не сохраняются и не влияют на выбор сессии для следующего resume.
- Повтор ID, равного последней durable activation attempt, является успешным no-op; тот же ID после другой activation добавляется в durable-порядок заново.
- Пока процесс агента работает, `session activate` принимается независимо от того, вызывал ли он ранее `attempt complete`.

## Вывод команд

- Lifecycle-сообщения имеют стабильную однострочную форму; human live-вывод и TUI agent board выводятся только в TTY и не являются машинным API.
- Human attempt без TTY не запускается: когда такая работа становится запускаемой, lifecycle-команда печатает диагностику в stderr, применяет runtime fail-fast protocol к уже работающим процессам и завершается с кодом `1`; headless-поведения для human Agent type нет.
- Пока работает human attempt, его процесс эксклюзивно занимает TTY, orchestrator не рисует TUI, а параллельные non-human attempts не пишут в терминал.
- Когда human attempt не работает и у команды есть TTY, orchestrator показывает agent board для запущенных non-human attempts: количество сообщений и последнее нормализованное сообщение из каждого Agent session view; конкретный layout TUI не является стабильным контрактом.
- Без TTY agent board не показывается; сырой output и event stream non-human процессов никогда не копируются в stdout, stderr или TTY, независимо от наличия human attempt.

- `start` печатает `workflow: <workflow-id>`, затем `Run <run-id>` в stdout и немедленно flush'ит обе строки до запуска первого процесса агента. К этому моменту materialized workflow, начальный attempt `0` и остальные обязательные данные run уже опубликованы durable.
- `validate` при успехе печатает `workflow <workflow-id>: valid` в stdout.
- Нормальное завершение печатает `run <run-id>: completed` в stdout.
- Явный пользовательский `/exit` распознаётся только Agent type работающего human attempt и печатает `run <run-id>: interrupted by user` и `resume: orchestrator resume <run-id>` в stdout; human attempt без completion остаётся незавершённым, после чего supervisor входит в user shutdown по правилам ниже.
- `resume` завершённого run не запускает агента, печатает `run <run-id>: already completed` и завершается с кодом `0`.
- Диагностика ошибок печатается в stderr и начинается с `error:`. Ошибка всегда содержит command context и ID известного run или workflow.
- Заблокированный частично удовлетворёнными dependencies run печатает отсутствующие source Steps в детерминированном порядке workflow и завершается с кодом `1`.
- После публикации RunId командой `start` либо разрешения существующего run командой `resume` любое управляемое завершение команды независимо от exit code последней строкой stdout печатает `Run <run-id> exited`; при `SIGKILL`, аварии процесса или отключении питания эта строка не гарантируется.

- Строка `Run <run-id>`, напечатанная `start`, является границей пользовательского обещания: после этой строки run можно продолжить из последнего полностью опубликованного durable-состояния.
- Если процесс погиб до строки, новый run пользователю не обещан; оставшийся зарезервированный каталог не переиспользуется другим `start`.

## `/exit` и user shutdown

- После `/exit` supervisor не запускает новые attempts, ready activations или native resume и не посылает сигнал уже работающим non-human процессам агентов; ожидающая работа остаётся для следующего явного `resume`.
- Control endpoint остаётся доступен работающим non-human процессам, а их возвраты, control calls и completion-кандидаты обрабатываются и финализируются по обычным правилам.
- Без нового termination signal supervisor ждёт самостоятельного возврата всех уже работающих non-human процессов без таймаута, затем закрывает endpoint, освобождает Run lock и завершает команду с `0`.
- `SIGINT`, включая Ctrl+C, `SIGTERM` или `SIGHUP` во время user shutdown переводит supervisor в signal shutdown по общим правилам ниже, посылает сигнал оставшимся процессам и заменяет код `0` кодом сигнала.

## Коды завершения

| Код | Значение |
| --- | --- |
| `0` | Команда выполнена, run уже завершён либо пользователь явно вышел через `/exit`. |
| `1` | Ошибка выполнения: Agent или Process failure, преждевременный Agent exit без completion, отсутствие TTY для запускаемой human-работы, blocked run, I/O failure или другая runtime error. |
| `2` | Ошибка синтаксиса CLI, неизвестная команда, ID недопустимого формата или отсутствие выбранного workflow. |
| `3` | Невалидный config, workflow, prompt template, durable run state, control event или запрос публикации artifact. |
| `4` | Указанный workflow, run или Agent не существует. |
| `5` | Run занят другим supervisor, control endpoint недоступен либо control context текущей обработки attempt уже закрыт. |
| `129` | Команда прервана `SIGHUP`. |
| `130` | Команда прервана `SIGINT`, включая Ctrl-C. |
| `143` | Команда прервана `SIGTERM`. |

- Ненулевой exit процесса агента не пробрасывается как код CLI и не сохраняется в Agent attempt record: до user shutdown текущая команда завершается с `1`, а во время user shutdown исходный код только показывается в диагностике и не заменяет итоговый код `/exit` `0`.
- Если одновременно работают несколько executors и supervisor ещё не вошёл в user shutdown, первая зафиксированная runtime error запускает fail-fast shutdown всей команды; в user shutdown возвраты уже работающих процессов не прерывают ожидание остальных.

- Запрос `attempt complete` с отсутствующим, повторяющимся или дополнительным InputId, не абсолютным или несуществующим path либо path, чей конечный объект не является regular file, не изменяет предыдущий кандидат; дочерний tool call завершается с `3`, и агент может исправить запрос.
- Каждый валидный `attempt complete`, принятый пока обработка attempt активна, завершается с `0`; одинаковый или изменённый повтор заменяет предыдущий кандидат целиком.
- Отдельной сущности `output` нет, а содержимое artifact обрабатывается по `format.spec.md`.

## `start`

- `start` принимает не более одного WorkflowId и любое число `--param <parameter-id>=<value>`; первое `=` отделяет ID от произвольного UTF-8 значения, пустое значение допустимо, а отсутствие `=` и повтор ParameterId являются syntax error с кодом `2` до создания run.
- После выбора workflow `start` требует ровно по одному значению каждого объявленного parameter и запрещает необъявленные parameters; нарушение является invalid workflow invocation с кодом `3` до создания run.
- До резервирования RunId команда загружает workflow и prompt templates, строит полный materialized workflow candidate в памяти и выполняет его полную validation без записи файлов run.
- Любая ошибка на этом этапе завершается с `2`, `3` или `4` и не создаёт run.

- После успешной проверки `start` атомарно резервирует свободный RunId.
- Коллизия имени повторяет резервирование с новым кандидатом и не является пользовательской ошибкой.
- Существующий каталог никогда не открывается как новый run, независимо от состояния его Run lock.

- После резервирования RunId команда получает Run lock, атомарно durable-публикует проверенный кандидат как `run/<run-id>/spec.yaml`, затем публикует initial attempt `0`; только после этого она печатает RunId.
- После печати запускается первый описанный Step. Ошибка запуска executor оставляет восстанавливаемый run и завершается с `1`.

## `resume`

- `resume` требует ровно один RunId и не выбирает run автоматически.

- Неизвестный RunId завершается с `4` без создания каталога.
- Занятый Run lock завершается с `5` до запуска executors и durable-изменений.
- Невалидная или противоречивая durable-модель завершается с `3` до запуска executors и durable-изменений.
- Завершённый run является успешным no-op с кодом `0`.
- Незавершённый валидный run продолжает attempts и ready activations по Rules в `features/lifecycle.feature` и `features/graph_execution.feature`.
- Blocked run без запускаемой работы завершается с `1` и перечисляет частично удовлетворённые dependencies.

- Временные файлы атомарной записи, artifacts без соответствующего attempt и файловые остатки attempt без completion не делают run повреждённым и игнорируются по Rule «Файловые остатки незавершённого attempt не входят в durable-модель» в `features/recovery.feature`.

## Read-only `run` inspection

- Каждая команда читает fingerprint durable entries до и после полной validation и повторяет изменившийся snapshot максимум четыре раза; стабильное противоречие завершается с `3`, а исчерпание retry при непрерывных изменениях — runtime code `1`, без partial stdout нового snapshot.
- `run watch` немедленно печатает initial show snapshot, затем проверяет состояние каждые 100 ms и печатает только изменившиеся validated snapshots; text snapshots следуют подряд как show documents, JSON использует по одному compact object на строку, terminal `blocked|completed` завершается с `0`, а signal — с общим кодом `129|130|143`.
- `run verify` с RunId проверяет только выбранный run; text содержит `run <id>: valid` либо `run <id>: invalid: <diagnostic>`, I/O даёт `1`, неизвестный явно выбранный run — `4`.
- Неизвестный явно выбранный RunId, attempt или InputId и artifact незавершённого attempt завершаются с `4`; синтаксически невалидный ID или attempt number завершается с `2`; противоречивая durable-модель завершается с `3`; до успешной полной проверки `run list`, `run show`, `run artifacts` и `run artifact` ничего не пишут в stdout.

## Source catalogs

- `workflow list`, `agent list` и `prompt list` являются read-only, разрешают только `--format text|json`, по умолчанию используют text и строят полный typed catalog до записи stdout.
- `workflow show` text печатает header `workflow <id>: path=<absolute-path>`, Agent Steps в прежней форме `step <id>: agent=<id|-> prompt=<id|-> human=<true|false> depends-on=<id,...|-> outputs=<id,...|->`, а Process Steps как `step <id>: process=<executable> args=<count> cwd=<cwd|-> stdout=<input-id|-> depends-on=<id,...|-> outputs=<id,...|->`; JSON возвращает object `{workflow,path,parameters,steps}`, где Step дополнительно содержит nullable `process`, отсутствующие optional references равны `null`, а source references и executable не разрешаются.
- `workflow plan` выполняет полный preflight выбранного WorkflowId; text после header печатает Agent Steps как прежде, а Process Steps как `step <id>: executor=process executable=<absolute-path> args=<count> cwd=<absolute-path> stdout=<input-id|-> depends-on=<id,...|-> outputs=<id,...|->`, JSON schema задана `format.spec.md`.
## `validate`

- Global I/O error во время `validate --all` прерывает команду с `1` без partial stdout.
- Полный контракт workflow graph и validation определён в `workflow.spec.md`.
- Ошибки выводятся в детерминированном порядке: сначала структура файла, затем steps в порядке workflow, затем ссылки и граф.

## Сигналы и закрытие терминала

- Первый `SIGINT`, `SIGTERM` или `SIGHUP`, полученный во время обычного выполнения либо user shutdown, переводит supervisor в signal shutdown:
    1. Новые attempts больше не создаются.
    2. Тот же сигнал передаётся всем supervised Agent и Process process groups.
    3. Control endpoint остаётся доступен, пока агенты завершаются. Уже принятые и поступившие от ещё supervised процессов tool calls, включая `session activate` и `attempt complete`, полностью сериализуются по обычным правилам. Parent завершает storage operation уже принятого запроса, даже если его дочерний `orchestrator` завершился до получения ответа.
    4. Supervisor ждёт завершения executors и Agent control calls не более 10 секунд.
    5. После timeout или второго termination signal оставшиеся process groups получают `SIGKILL` и reaped.
    6. Supervisor завершает уже принятую атомарную storage operation и финализирует кандидаты агентов, чьи процессы вернули управление, закрывает control endpoint и освобождает Run lock.

- Команда возвращает `128 + signal`: `130` для Ctrl-C/SIGINT, `143` для SIGTERM и `129` для SIGHUP. Эскалация по timeout или повторному сигналу не меняет код первого сигнала.

- Runtime fail-fast использует тот же shutdown protocol, передавая `SIGTERM` оставшимся executors, но завершается с кодом `1`.

- При `SIGKILL`, аварии процесса, отключении питания или kill во время системного вызова graceful shutdown невозможен.
- Ядро освобождает Run lock, атомарно публикуемые временные файлы игнорируются, а следующий `resume` видит только полностью подтверждённые durable events и завершённые attempts.
- Вызов `attempt complete`, не получивший успешного ответа, можно безопасно повторить, пока текущая обработка attempt остаётся активной; повтор целиком заменяет неопределённый предыдущий кандидат.
- Resume не обещает восстановить volatile in-memory state, временные файлы агента или неподтверждённую работу.
