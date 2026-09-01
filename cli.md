# CLI-контракт orchestrator

Этот документ задаёт публичное и agent-facing поведение CLI: команды, поиск конфигурации, вывод, коды завершения и обработку прерываний. Модель run, attempts, artifacts и workflow определена в `SPEC.md`, а layout состояния и форматы файлов — в `format.spec.md`.

## Команды и аргументы

- `orchestrator start [<workflow-id>]` создаёт новый run выбранного workflow.
- `orchestrator resume <run-id>` продолжает только явно указанный run. Выбор последнего run и другие формы implicit resume не поддерживаются.
- `orchestrator run list [--state active|blocked|completed]... [--workflow <workflow-id>] [--format text|json]` читает и перечисляет все durable runs без запуска Agent или изменения состояния.
- `orchestrator run show <run-id> [--format text|json]` читает один durable run и показывает его materialized Steps, attempts и вычисленный frontier.
- `orchestrator run artifacts <run-id> [--format text|json]` перечисляет опубликованные версии durable artifacts.
- `orchestrator run artifact <run-id> <attempt-n> <input-id>` пишет в stdout точные bytes опубликованного artifact выбранного attempt.
- `orchestrator run watch <run-id> [--format text|json]` наблюдает изменения validated snapshot до terminal state или signal.
- `orchestrator run verify [<run-id>] [--format text|json]` формирует полный read-only validation report.
- `orchestrator workflow list [--format text|json]` перечисляет source workflow templates.
- `orchestrator workflow show <workflow-id> [--format text|json]` показывает структуру выбранного source workflow template.
- `orchestrator workflow graph <workflow-id> [--format text|json]` показывает validated dependency graph выбранного source workflow.
- `orchestrator workflow plan <workflow-id> [--format text|json]` показывает полностью materialized execution plan без создания run.
- `orchestrator agent list [--format text|json]` перечисляет named Agents из validated config.
- `orchestrator agent show <agent-id> [--format text|json]` показывает выбранного named Agent из validated config.
- `orchestrator prompt list [--format text|json]` перечисляет source prompt templates.
- `orchestrator prompt show <prompt-id> [--format text|json]` читает выбранный source prompt template.
- `orchestrator validate [<workflow-id>]` проверяет workflow без создания или изменения run.
- `orchestrator validate --all [--format text|json]` проверяет все source workflow templates и формирует полный отчёт.
- `orchestrator config get <key>` читает одно значение конфигурации.
- `orchestrator config set <key> <value>` атомарно изменяет одно значение.
- `orchestrator config list` печатает все поддерживаемые значения конфигурации.
- `orchestrator attempt complete [--artifact <input-id> <path>]...` передаёт полный текущий кандидат completion и artifacts; attempt завершается только после возврата процесса агента.
- `orchestrator session activate <session-id>` фиксирует activation внутренней сессии текущего attempt.
Файлы config, workflow, prompt templates и runs размещаются только по layout из `format.spec.md`. CLI не ищет их в текущем каталоге и не применяет fallback-пути.

## Корень состояния и переменные окружения

- Пути из `format.spec.md` отсчитываются от корня состояния. Корень по умолчанию задан там и разрешается через `HOME`.
- `ORC_HOME` заменяет корень целиком: при непустом значении используются только пути относительно него по `format.spec.md`, а корень по умолчанию не читается и не изменяется. Значение обязано быть абсолютным путём, иначе команда завершается кодом `3` без чтения и изменения состояния.
- Замена корня не добавляет альтернативных мест поиска: правило об отсутствии поиска в текущем каталоге и fallback-путей действует и для `ORC_HOME`.
- `ORC_AGENT_COMMAND` задаёт исполняемый файл сразу для всех Agent type. При непустом значении запускается именно он и поиск по `PATH` не выполняется. Значение обязано быть абсолютным путём существующего executable regular file, иначе команда завершается кодом `3` до запуска агента и без изменения run.
- Замена исполняемого файла не меняет остальное поведение type: аргументы, environment запуска и интерпретация протокола остаются его собственными по разделам Agent type в `SPEC.md`; дополнительный type-ID argument не добавляется, а fake executable различает adapter по Codex или Claude CLI-аргументам.
- Каждый Agent process получает `ORC_STEP_ID`, `ORC_CONTROL_ENDPOINT`, `ORC_RUN_ID`, `ORC_ATTEMPT` и полный YAML input mapping в `ORC_INPUT`; эти переменные не заменяют type-specific prompt, model, reasoning и native resume arguments.
- Supervisor передаёт эти переменные дочернему `orchestrator` без изменений, поэтому `attempt complete` и `session activate` работают в том же корне и с тем же выбором исполняемых файлов. Дочерний процесс их не переопределяет.

## Выбор workflow

`start` и `validate` используют один resolver:

1. Явно переданный WorkflowId выбирается без чтения `default-workflow` и других workflow templates. Config всё равно читается позднее для разрешения Agents и `default-agent`.
2. Без аргумента выбирается только `default-workflow` из config; если поле не задано, команда просит передать WorkflowId или настроить default и завершается с `2`.
3. Отсутствующий template выбранного workflow завершает команду с `4`.

Config обязан соответствовать `format.spec.md`. Невалидный config завершает команду с `3`. После выбора по аргументу либо `default-workflow` команды полностью materialize’ят в памяти и валидируют кандидат workflow вместе со всеми выбранными Agents и используемыми prompt templates по `workflow.spec.md`; остальные workflow templates не читаются и не валидируются. Если `default-workflow` ссылается на отсутствующий workflow, команда завершается с `4` и не переключается на другой workflow.

## `config get`, `config set` и `config list`

Поддерживаются ключи `default-workflow`, `default-agent` и `max-parallel-agents`. Именованные Agents редактируются непосредственно в config; `config set` не создаёт и не изменяет их конфигурацию.

- Каждый `config set` сначала формирует и целиком валидирует кандидат config, а публикует его атомарно только после успеха проверки; workflow materialize’ится и валидируется не при изменении config, а после его выбора командами `start` и `validate`.

- `orchestrator config get default-workflow` печатает WorkflowId без имени ключа или `null`, если default не задан.
- `orchestrator config get default-agent` печатает AgentId без имени ключа или `null`, если default не задан.
- `orchestrator config get max-parallel-agents` печатает настроенный положительный integer или эффективное значение `5`, если поле отсутствует.
- `orchestrator config list` печатает в фиксированном порядке `default-workflow: <workflow-id|null>`, `default-agent: <agent-id|null>` и `max-parallel-agents: <positive-integer>`. Команда не перечисляет workflow-файлы или именованные Agents.
- `orchestrator config set default-workflow <workflow-id>` требует существующий workflow template, заменяет поле кандидата и при успехе общей проверки печатает `default-workflow: <workflow-id>`.
- `orchestrator config set default-agent <agent-id>` требует существующий named Agent, заменяет поле кандидата и при успехе общей проверки печатает `default-agent: <agent-id>`.
- `orchestrator config set max-parallel-agents <positive-integer>` требует положительное значение, заменяет поле кандидата и при успехе общей проверки печатает `max-parallel-agents: <positive-integer>`.

Отсутствующий config эквивалентен unset workflow и Agent defaults, эффективному `max-parallel-agents: 5` и пустому набору Agents: `get` и `list` завершаются с `0`. Неизвестный ключ или неверное число аргументов завершается с `2`. Невалидный кандидат config или неположительное значение `max-parallel-agents` завершается с `3`, а отсутствие workflow или Agent, прямо указанного аргументом `config set`, — с `4`; любая ошибка оставляет config неизменным. Config commands не получают run locks и не изменяют существующие runs; конкурентные успешные `set` публикуют целый файл атомарно, и остаётся значение последнего durable commit.

## Agent-facing команды

- `attempt complete` и `session activate` предназначены для tool calls агента и agent-specific hooks.
- Они получают RunId, номер attempt и адрес parent supervisor только из control context текущего процесса; передать или выбрать их аргументами нельзя.
- При отсутствующем или устаревшем endpoint либо после того, как parent наблюдал возврат процесса агента, обрабатывавшего attempt, команда завершается с `5` и не пишет в каталог run напрямую.

- `orchestrator attempt complete` принимает ноль или больше повторяющихся `--artifact <input-id> <path>`.
- Набор InputIds обязан в точности совпадать с `outputs` текущего Step; для пустого `outputs` аргументы `--artifact` не передаются.
- Каждый `<path>` является переданным агентом абсолютным путём к source-файлу artifact; отдельный временный каталог orchestrator не создаёт и не передаёт, containment пути не проверяется, symbolic links разрешаются операционной системой, а конечный объект обязан быть regular file.
- Parent сначала проверяет и полностью читает все файлы, затем целиком заменяет ими последний принятый кандидат текущей обработки attempt.
- Каждый следующий валидный `attempt complete`, принятый пока обработка attempt активна, может передать другие bytes artifacts и снова целиком заменяет предыдущий кандидат.
- До возврата процесса агента кандидат не завершает attempt и его artifacts не доступны workflow graph; при возврате последний принятый кандидат публикуется по правилам `SPEC.md`.

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
| `1` | Ошибка выполнения: agent failure, преждевременный exit без completion, отсутствие TTY для запускаемой human-работы, blocked run, I/O failure или другая runtime error. |
| `2` | Ошибка синтаксиса CLI, неизвестная команда, ID недопустимого формата или отсутствие выбранного workflow. |
| `3` | Невалидный config, workflow, prompt template, durable run state, control event или запрос публикации artifact. |
| `4` | Указанный workflow, run или Agent не существует. |
| `5` | Run занят другим supervisor, control endpoint недоступен либо control context текущей обработки attempt уже закрыт. |
| `129` | Команда прервана `SIGHUP`. |
| `130` | Команда прервана `SIGINT`, включая Ctrl-C. |
| `143` | Команда прервана `SIGTERM`. |

- Ненулевой exit процесса агента не пробрасывается как код CLI и не сохраняется в Agent attempt record: до user shutdown текущая команда завершается с `1`, а во время user shutdown исходный код только показывается в диагностике и не заменяет итоговый код `/exit` `0`.
- Если одновременно работают несколько агентов и supervisor ещё не вошёл в user shutdown, первая зафиксированная runtime error запускает fail-fast shutdown всей команды; в user shutdown возвраты уже работающих процессов не прерывают ожидание остальных.

- Запрос `attempt complete` с отсутствующим, повторяющимся или дополнительным InputId, не абсолютным или несуществующим path либо path, чей конечный объект не является regular file, не изменяет предыдущий кандидат; дочерний tool call завершается с `3`, и агент может исправить запрос.
- Каждый валидный `attempt complete`, принятый пока обработка attempt активна, завершается с `0`; одинаковый или изменённый повтор заменяет предыдущий кандидат целиком.
- Отдельной сущности `output` нет, а содержимое artifact обрабатывается по `format.spec.md`.

## `start`

- `start` принимает не более одного WorkflowId и разрешает его по общим правилам.
- До резервирования RunId команда загружает workflow и prompt templates, строит полный materialized workflow candidate в памяти и выполняет его полную validation без записи файлов run.
- Любая ошибка на этом этапе завершается с `2`, `3` или `4` и не создаёт run.

- После успешной проверки `start` атомарно резервирует свободный timestamp RunId.
- Коллизия имени повторяется с новым timestamp и не является пользовательской ошибкой.
- Существующий каталог никогда не открывается как новый run, независимо от состояния его Run lock.

- После резервирования RunId команда получает Run lock, атомарно durable-публикует проверенный кандидат как `run/<run-id>/spec.yaml`, затем публикует initial attempt `0`; только после этого она печатает RunId.
- После печати запускается первый описанный step. Ошибка запуска агента оставляет восстанавливаемый run и завершается с `1`.

## `resume`

- `resume` требует ровно один RunId и не выбирает run автоматически.

- Неизвестный RunId завершается с `4` без создания каталога.
- Занятый Run lock завершается с `5` до запуска агентов и durable-изменений.
- Невалидная или противоречивая durable-модель завершается с `3` до запуска агентов и durable-изменений.
- Завершённый run является успешным no-op с кодом `0`.
- Незавершённый валидный run продолжает attempts и ready activations по правилам `SPEC.md`.
- Blocked run без запускаемой работы завершается с `1` и перечисляет частично удовлетворённые dependencies.

- Временные файлы атомарной записи, artifacts без соответствующего attempt и файловые остатки attempt без completion не делают run повреждённым и игнорируются по правилам `SPEC.md`.

## Read-only `run` inspection

- Все inspection-команды являются read-only: они не получают Run lock, не создают control endpoint, не запускают Agent и не создают или изменяют файлы; занятый supervisor'ом run разрешено читать.
- Каждая команда читает fingerprint durable entries до и после полной validation и повторяет изменившийся snapshot максимум четыре раза; стабильное противоречие завершается с `3`, а исчерпание retry при непрерывных изменениях — runtime code `1`, без partial stdout нового snapshot.
- `run list` игнорирует нечисловые entries в `run/`, проверяет каждый числовой каталог как полную durable-модель и печатает по одной строке в порядке возрастания RunId: `run <run-id>: workflow=<workflow-id> state=<active|blocked|completed>`; отсутствие каталога `run/` или runs является успешным пустым выводом.
- Повторяемые `--state` объединяются как OR, `--workflow` соединяется со state как AND, одинаковые фильтры идемпотентны, а validation выполняется для всех runs до фильтрации; невалидные state, format или WorkflowId завершаются с `2` до чтения runs.
- Производное состояние `active` означает наличие незавершённого attempt или непустого ready frontier и не утверждает, что сейчас существует процесс Agent; `completed` означает завершённую модель без нового frontier, остальные валидные состояния являются `blocked`.
- `run show` первой строкой печатает ту же summary-строку, затем для каждого Step в порядке materialized workflow строку `step <step-id>: attempts=<n,...|->`, затем attempts в порядке глобального номера строками `attempt <n>: step=<step-id> state=<active|completed> session=<session-id|-> input=<n,...|->` и последней строкой `frontier: ready=<step-id,...|-> missing=<step-id,...|->`.
- `run artifacts` после полной validation печатает completed artifacts в порядке attempt и outputs Step строками `artifact <attempt>: step=<step-id> input=<input-id> bytes=<n> path=<absolute-path>`; orphan, temporary и outputs незавершённых attempts не выводятся, пустой набор успешен.
- `run artifact` принимает десятичный глобальный attempt number и kebab-case InputId, требует существующий completed attempt и объявленный для его Step output, полностью проверяет durable run до открытия artifact и копирует файл в stdout без текстового преобразования или добавления newline.
- `--format text` является default и сохраняет описанный text output; JSON schema использует snake_case fields, `run list` и `run artifacts` возвращают arrays, `run show` возвращает object `{run_id,workflow,state,steps,attempts,frontier,artifacts}`, числа остаются numbers, отсутствующая session — `null`, arrays сохраняют deterministic durable order.
- `run watch` немедленно печатает initial show snapshot, затем проверяет состояние каждые 100 ms и печатает только изменившиеся validated snapshots; text snapshots следуют подряд как show documents, JSON использует по одному compact object на строку, terminal `blocked|completed` завершается с `0`, а signal — с общим кодом `129|130|143`.
- `run verify` без RunId проверяет все числовые каталоги, с RunId — только выбранный; text содержит `run <id>: valid` либо `run <id>: invalid: <diagnostic>`, JSON имеет форму `{runs:[{run_id,valid,diagnostics}]}`, наличие invalid даёт `3` после полного stdout report, I/O даёт `1`, неизвестный явно выбранный run — `4`.
- Неизвестный явно выбранный RunId, attempt или InputId и artifact незавершённого attempt завершаются с `4`; синтаксически невалидный ID или attempt number завершается с `2`; противоречивая durable-модель завершается с `3`; до успешной полной проверки `run list`, `run show`, `run artifacts` и `run artifact` ничего не пишут в stdout.

## Source catalogs

- `workflow list`, `agent list` и `prompt list` являются read-only, разрешают только `--format text|json`, по умолчанию используют text и строят полный typed catalog до записи stdout.
- `workflow list` сортирует templates по WorkflowId; text печатает только `<workflow-id>` по одному на строку, JSON — array объектов `{workflow,path}` с абсолютным path; отсутствие `workflow/` успешно и даёт пустой text либо `[]`.
- `agent list` полностью проверяет `config.yaml` и сортирует named Agents по AgentId; text печатает `agent <id>: type=<type> model=<model> reasoning=<reasoning>`, JSON — array объектов `{agent,type,model,reasoning}`; отсутствие config или пустой `agents` mapping успешно.
- `prompt list` сортирует templates по PromptId; text печатает `prompt <id>: bytes=<n> path=<absolute-path>`, JSON — array объектов `{prompt,bytes,path}`; `bytes` является JSON number, отсутствие `prompt/` успешно.
- `workflow show` text печатает header `workflow <id>: path=<absolute-path>`, затем Steps в source order строками `step <id>: agent=<id|-> prompt=<id|-> human=<true|false> depends-on=<id,...|-> outputs=<id,...|->`; JSON возвращает object `{workflow,path,steps}`, где Step содержит `{id,agent,prompt,human,depends_on,outputs}`, отсутствующие optional references равны `null`, а source references не разрешаются через config, prompts или graph.
- `workflow graph` не читает config и prompts; text печатает `workflow <id>: path=<absolute-path>`, `bootstrap: <first-step-id>` и dependency edges `<dependency> -> <step>` в source order, JSON возвращает object `{workflow,path,bootstrap,nodes,edges}` с edge objects `{from,to}`.
- `workflow plan` выполняет полный preflight выбранного WorkflowId; text печатает `workflow <id>: max-parallel-agents=<n>` и Steps строками `step <id>: type=<type> model=<model> reasoning=<reasoning> prompt-bytes=<n|-> human=<true|false> depends-on=<id,...|-> outputs=<id,...|->`, JSON schema задана `format.spec.md`.
- `agent show` выполняет полную config validation и затем выбирает Agent; text печатает `agent <id>: type=<type> model=<model> reasoning=<reasoning>`, JSON возвращает object `{agent,type,model,reasoning}`.
- `prompt show` text побайтово равен UTF-8 содержимому выбранного template без добавления newline, JSON возвращает object `{prompt,bytes,path,content}` с числовым `bytes` и точным `content`; команда не читает соседние templates.
- Невалидный basename contract file для list, non-regular выбранный или перечисляемый template, non-UTF-8 прочитанный prompt, невалидный source workflow либо невалидный config завершается с `3` без partial stdout; неизвестный явно выбранный source ID даёт `4`, синтаксически невалидный ID или format отклоняется с `2` до чтения source; другие расширения и начинающиеся с `.` временные entries list-команд игнорируются.

## `validate`

- `validate` принимает не более одного WorkflowId, разрешает его по общим правилам и выполняет те же проверки, что preflight команды `start`.
- `validate --all` взаимоисключаем с WorkflowId, сортирует source templates по WorkflowId и возвращает text `workflow <id>: valid|invalid: <diagnostic>` либо JSON `{workflows:[{workflow,valid,diagnostics}]}`; пустой catalog успешен, все valid дают `0`, наличие invalid даёт `3` после полного stdout, а global I/O error прерывает команду с `1` без partial stdout.
- Полный контракт workflow graph и validation определён в `workflow.spec.md`.
- Ошибки выводятся в детерминированном порядке: сначала структура файла, затем steps в порядке workflow, затем ссылки и граф.
- Невалидный workflow завершается с `3`.

## Сигналы и закрытие терминала

- Первый `SIGINT`, `SIGTERM` или `SIGHUP`, полученный во время обычного выполнения либо user shutdown, переводит supervisor в signal shutdown:
    1. Новые attempts больше не создаются.
    2. Тот же сигнал передаётся всем supervised agent process groups.
    3. Control endpoint остаётся доступен, пока агенты завершаются. Уже принятые и поступившие от ещё supervised процессов tool calls, включая `session activate` и `attempt complete`, полностью сериализуются по обычным правилам. Parent завершает storage operation уже принятого запроса, даже если его дочерний `orchestrator` завершился до получения ответа.
    4. Supervisor ждёт завершения агентов и control calls не более 10 секунд.
    5. После timeout или второго termination signal оставшиеся agent process groups получают `SIGKILL` и reaped.
    6. Supervisor завершает уже принятую атомарную storage operation и финализирует кандидаты агентов, чьи процессы вернули управление, закрывает control endpoint и освобождает Run lock.

- Команда возвращает `128 + signal`: `130` для Ctrl-C/SIGINT, `143` для SIGTERM и `129` для SIGHUP. Эскалация по timeout или повторному сигналу не меняет код первого сигнала.

- Runtime fail-fast использует тот же shutdown protocol, передавая `SIGTERM` оставшимся агентам, но завершается с кодом `1`.

- При `SIGKILL`, аварии процесса, отключении питания или kill во время системного вызова graceful shutdown невозможен.
- Ядро освобождает Run lock, атомарно публикуемые временные файлы игнорируются, а следующий `resume` видит только полностью подтверждённые durable events и завершённые attempts.
- Вызов `attempt complete`, не получивший успешного ответа, можно безопасно повторить, пока текущая обработка attempt остаётся активной; повтор целиком заменяет неопределённый предыдущий кандидат.
- Resume не обещает восстановить volatile in-memory state, временные файлы агента или неподтверждённую работу.
