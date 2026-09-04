# Orchestrator: целевая спецификация

## Назначение

`orchestrator` — CLI для выполнения длинных задач агентами и обычными процессами по заранее заданному
workflow. Выполнение сохраняется как run: его можно безопасно прервать,
продолжить в другом процессе и восстановить без контекста предыдущего запуска.
Публичные команды, вывод, коды завершения и обработка сигналов определены в
`cli.md`. Workflow graph, его validation и scheduling определены в
`workflow.spec.md`. Layout состояния, имена и содержимое файлов определены в
`format.spec.md`.

## Сущности

- **Run** — один сохраняемый запуск workflow. Уникальность обеспечивает атомарное резервирование каталога run. Run хранит материализованный workflow, записи фактов attempts и опубликованные артефакты по `format.spec.md`. Текущая позиция и состояние run отдельно не сохраняются: они вычисляются из этой модели.
- **Agent type** — встроенная реализация интерфейса `AgentType`; это не запись config. Каждая реализация строит command и environment запуска из materialized Agent, input mapping, prompt и control context, выполняет native resume и интерпретирует протокол и завершение процесса. Путь исполняемого файла type переопределяется только переменной окружения из `cli.md`; это не меняет ни построение аргументов и environment запуска, ни интерпретацию протокола. Для одного и того же materialized workflow и валидного Agent attempt record он всегда детерминированно выводит один и тот же результат: успешное завершение шага либо необходимость продолжения attempt. Неизвестные или противоречивые agent-specific факты являются ошибкой. Успешное завершение требует, чтобы процесс агента вернул управление supervisor с принятым кандидатом completion и полным набором artifacts; возврат процесса без такого кандидата сам по себе успехом не является. В human-режиме процесс агента напрямую и эксклюзивно занимает TTY команды, а orchestrator не проксирует его stdin через PTY. В non-human режиме Agent type запускает агента без интерактивного терминала, читает его output или event stream для распознавания протокола и Agent session view, но не пересылает сырой поток в TTY, stdout или stderr orchestrator.
- **Agent** — именованная конфигурация агента. Agent выбирается Step или `default-agent` и materialize’ится в кандидат workflow до создания run; проверенный кандидат durable-публикуется после резервирования run. Поэтому последующее изменение config не меняет Agent существующего run или его resume.
- **Attempt** — одна обработка конкретной Step activation выбранным Agent или Process executor. Agent attempt могут последовательно обрабатывать несколько процессов агента через native resume, а незавершённый Process attempt повторно запускает materialized command at-least-once. Attempt создаётся атомарной публикацией attempt record формата `format.spec.md`; полный input mapping после первой публикации не изменяется, recovery-critical события заменяют record атомарно, производные статусы не сохраняются, продолжение использует тот же номер, и номер никогда не назначается другому attempt.
- **Process executor** — materialized executable, cwd, argv templates и optional stdout output обычного non-human Step; Process не является Agent type, не имеет native session или control context и завершает attempt только через проверенный exit code и supervisor-owned artifact commit.
- **Agent session activation** — сообщение об активации внутренней сессии агента с её native session ID. Создание, продолжение и fork имеют одну семантику и не сохраняются как разные виды событий; при fork активируется дочерняя сессия. Agent type получает activation напрямую из протокола запущенного процесса, если это возможно. Agent-specific hook используется только когда native session ID иначе недоступен: он запускает дочерний процесс `orchestrator`, который передаёт событие parent supervisor по control endpoint. Активация не создаёт новый orchestrator Run.
- **Agent session view** — volatile-представление активной внутренней сессии агента для non-human attempt: количество полученных сообщений и последнее сообщение, нормализованное в одну строку. Agent type получает сообщения из output или event stream запущенного процесса; сырой поток после интерпретации отбрасывается, а view используется только для TUI agent board и не записывается в Agent attempt record, поскольку не участвует в recovery или вычислении готовности steps.
- **Artifact** — версионируемый сформированный результат с durable-ключом `(attempt-n, step-id, input-id)`. Правила его использования графом заданы в `workflow.spec.md`, а файловый формат — в `format.spec.md`. Artifact входит в durable-модель и становится доступен графу и неизменяем только при финализации последнего кандидата completion после возврата процесса агента.
- **Run lock** — эксклюзивная блокировка одного run. Пока её удерживает процесс `orchestrator`, второй процесс не может запустить тот же run; разные runs могут выполняться одновременно. Блокировка ядра берётся на lock-файле из `format.spec.md`; наличие файла само по себе не означает активность.

## Native resume

Продолжение run выполняется только явно с обязательным RunId; implicit resume
последнего run не поддерживается. Точный CLI-контракт задан в `cli.md`. После
выхода процесса без completion event orchestrator не запускает тот же attempt
повторно в рамках текущей команды.

При явном resume незавершённый attempt продолжает ту же внутреннюю сессию,
которая указана последней Agent session activation в durable-порядке
Agent attempt record. Причина её появления — create, resume или fork — не
сохраняется и не даёт ей отдельного приоритета. Если activations нет, Agent type
создаёт новую внутреннюю сессию для того же attempt и того же `n`.

Fallback с native resume на новую сессию запрещён. Agent type без поддержки
native resume отклоняется при `validate` и `start`; если такая несовместимость
обнаружена при `resume` существующего run, команда завершается fail-fast до
запуска агента и без интерактивных вопросов.

Process Step не имеет native session и не создаёт session activation. Если его attempt остался без `completed`, последующий явный `resume` повторно запускает тот же materialized executable с теми же cwd, параметрами и разрешёнными из durable input mapping аргументами; это at-least-once execution, поэтому внешние side effects Process обязаны быть идемпотентными.

Ненулевой exit code процесса не сохраняется в Agent attempt record и до входа supervisor в user shutdown fail-fast завершает текущую команду: новые attempts и автоматический native resume не запускаются. В user shutdown возврат одного из уже работающих non-human процессов не прерывает ожидание остальных. При возврате процесса наличие последнего принятого кандидата completion определяет состояние attempt независимо от exit code: кандидат финализируется терминальным событием `completed`, а без кандидата durable-событие не добавляется и attempt может быть продолжен только последующим явным `orchestrator resume <run-id>`.

Human attempt запускается только с TTY lifecycle-команды, который процесс агента занимает напрямую и эксклюзивно; если human-работа становится запускаемой без TTY, supervisor не создаёт для неё процесс и завершает команду runtime fail-fast с кодом `1`. Headless-режима для human Agent type нет.

## Control endpoint и события

Supervisor, удерживающий Run lock, создаёт один volatile Unix domain socket
для всех дочерних процессов `orchestrator` данного run. Socket доступен только
владельцу каталога run и существует только во время работы supervisor; он не
является частью durable-модели. Один listener обслуживает краткоживущие запросы,
отдельные постоянные файловые дескрипторы для attempts или activations не
удерживаются.

Перед каждым запуском процесса агента supervisor передаёт через environment
endpoint, RunId и номер attempt. Дочерний `orchestrator`, вызванный hook или
обычным tool call, берёт этот контекст из environment; переданные самим агентом
RunId или номер attempt не считаются доверенными. В payload `session activate`
он передаёт native session ID. В payload `attempt complete` он передаёт
completion и список пар из `input-id` и абсолютного пути к source-файлу. Точный
синтаксис обеих команд определён в `cli.md`.

Новый control call parent принимает только если контекст указывает на обслуживаемый run и существующий attempt, который сейчас обрабатывается процессом агента под управлением этого supervisor: supervisor уже запустил процесс и ещё не наблюдал его возврат. Идентичность дочернего процесса, отправившего call, в control context не передаётся и отдельно не проверяется. Повторный `attempt complete` принимается только пока эта обработка attempt активна и заменяет её предыдущий кандидат.
Права Unix socket защищают endpoint от других локальных пользователей. Защита
от процесса, уже выполняющегося с теми же правами пользователя, не входит в
текущий scope.

Новая session activation сравнивается только с последней durable session
activation данного attempt. Если native session ID совпадает, запрос возвращает
прежний успех без изменения Agent attempt record; иначе ID добавляется в конец
списка.
Поэтому последовательность `a → b → a → a` сохраняется как `a → b → a`.
Повтор ID после другого ID является новой activation: порядок `a → b → a` сохраняет выбор `a` для следующего native resume. Вид появления session — create, resume или fork — на это правило не влияет. Отдельный идентификатор события не создаётся. Вызовы `attempt complete` не закрывают активную обработку attempt для новых activations и каждый раз целиком заменяют её текущий кандидат completion.

Parent сериализует новые принятые session activations, добавляя их в упорядоченный список фактов Agent attempt record в порядке durable commit. Позиция среди `session-activated` определяет «последнюю» Agent session activation независимо от других видов событий. Parent сначала атомарно заменяет record, затем обновляет in-memory состояние и только после этого отвечает дочернему процессу успехом. Кандидат completion остаётся volatile до возврата процесса и после crash не восстанавливается из Agent attempt record.

Parent сериализует новые control calls с наблюдением возврата процесса агента: запрос, упорядоченный раньше возврата, полностью обрабатывается до финализации кандидата и закрытия control context, а упорядоченный после возврата отклоняется по правилам `cli.md`. Пока обработка attempt активна, каждый валидный `attempt complete` заменяет предыдущий кандидат независимо от его содержимого, а новые session activations продолжают приниматься.

Если endpoint недоступен, control context невалиден или parent завершился, дочерний `orchestrator` возвращает ненулевой exit code и не пишет файлы run напрямую. Без успешного ответа caller не может определить, принят ли кандидат. `attempt complete` можно безопасно повторить, пока обработка attempt активна: любой валидный повтор целиком заменяет предыдущий кандидат. Для `session activate` retry не является частью контракта.

После crash новый supervisor сначала получает Run lock, удаляет оставшийся
volatile socket path и создаёт endpoint с новым, не переиспользуемым путём.
Поэтому дочерние процессы со старым endpoint не могут отправлять события после
resume.

## Публикация artifacts и completion

Каждым вызовом `orchestrator attempt complete` агент передаёт по одной паре из `input-id` и абсолютного пути к source-файлу для каждого объявленного output текущего Step; при пустом `outputs` передаётся ноль пар. Orchestrator не создаёт и не передаёт агенту отдельный временный каталог, не требует containment source-путей и разрешает symbolic links по правилам операционной системы; конечный объект каждого пути обязан быть regular file. Source-файлы не входят в durable-модель run и должны сохраняться caller'ом до успешного ответа tool call.

Дочерний `orchestrator` берёт RunId и номер attempt из control context и передаёт один запрос parent supervisor. Он не создаёт и не изменяет файлы внутри каталога run. Parent проверяет весь запрос: InputIds совпадают с объявленными InputIds без пропусков и дополнений, каждый path абсолютен, существует и после разрешения symbolic links указывает на regular file. Затем parent полностью читает файлы как непрозрачные bytes и целиком заменяет ими volatile-кандидат текущей обработки attempt; успешный ответ подтверждает принятие кандидата, но не завершение attempt и не публикацию artifacts в workflow graph.

Когда процесс агента возвращает управление, parent сначала обрабатывает все ранее принятые control calls, затем при наличии кандидата публикует его полный набор под durable-именами и одной атомарной заменой Agent attempt record добавляет терминальное событие `completed`. Эта замена record является commit-точкой успешного attempt и делает artifacts частью durable-модели и графа; без кандидата Agent attempt record не изменяется и attempt остаётся незавершённым.

Crash до commit-точки может оставить файлы под durable-именами, но без терминального события `completed` они не являются artifacts модели и при восстановлении игнорируются. Volatile-кандидат после crash не восстанавливается; следующая обработка того же attempt может передать новый. После commit набор и содержимое artifacts неизменяемы, а control context завершённой обработки больше не принимает вызовы.

Parent supervisor является единственным поддерживаемым писателем artifact-файлов
внутри run. Прямая запись агентом или другим процессом нарушает storage-контракт;
защита и обнаружение изменений со стороны процесса того же локального
пользователя не входят в текущий scope.

## Process executor

Перед запуском Process supervisor создаёт уникальный staging regular-file path для каждого объявленного output, передаёт mapping в `ORC_OUTPUT` и разрешает `{{output:…}}` в argv в соответствующий path; Process может создать или заменить этот файл, но не пишет durable artifact напрямую.

Process наследует environment supervisor с заменой `ORC_STEP_ID`, `ORC_RUN_ID`, `ORC_ATTEMPT`, `ORC_INPUT` и `ORC_OUTPUT`, запускается отдельной process group с `stdin: null`, наследуемым stderr и stdout согласно `stdout`; `stdout: <input-id>` направляет точные bytes stdout в staging path этого output, а без `stdout` stdout наследуется.

После exit code `0` supervisor проверяет, что каждый объявленный output существует и является regular file, полностью читает staging files и публикует artifacts и `completed` через ту же commit-точку, что Agent completion; отсутствующий или невалидный output является runtime failure без terminal event.

Ненулевой exit code Process является runtime failure, не публикует staging outputs и не добавляет terminal event; код процесса не становится exit code orchestrator.

## Блокировка run

Перед запуском первого шага через `start` или `resume` процесс открывает lock-файл
выбранного run и неблокирующе получает exclusive kernel lock.
Занятый lock завершает команду с ошибкой до запуска агента.

При `start` процесс получает timestamp-кандидат и атомарно резервирует каталог
run из layout `format.spec.md` только при отсутствии такого каталога. Победитель
сразу получает RunId. Если каталог уже существует, кандидат не переиспользуется:
`start` дожидается другого значения timestamp и повторяет атомарное
резервирование. Существующий каталог никогда не присоединяется к новому `start`.
Ошибка резервирования, отличная от коллизии имени, завершает `start`.

После успешного резервирования новый run захватывает собственный Run lock
по общим правилам выполнения. Этот lock не участвует в выборе RunId и нужен
только для запрета одновременного выполнения уже созданного run.

Любой шаг разрешено запускать только supervisor, уже удерживающему
Run lock. Между последовательными запусками steps lock не освобождается и
повторно не захватывается. Поэтому весь непрерывный проход по workflow защищён
одной блокировкой.

Ядро освобождает Run lock при штатном выходе, crash или завершении
процесса. GC и другие операции перемещения или удаления run не входят в
текущий scope.

## Создание и восстановление Agent attempt

Атомарная публикация Agent attempt record является единственной commit-точкой
создания Agent attempt. До неё attempt не существует. Следующий
новый attempt любого step получает `0`, если во всём run ещё нет опубликованных
attempt records, иначе на единицу больше максимального использованного во всём
run `n`; пропуски не заполняются. Так artifact, оставшийся после crash до
создания attempt, не вызывает повторного использования своего имени.

Отдельной commit-точки в виде `output` нет. После получения Run lock
orchestrator читает факты из каждого Agent attempt record, а соответствующий
Agent type выводит из них и materialized workflow, завершён ли attempt успешно
или требует продолжения:

| Durable-состояние | Значение | Действие при `resume` |
| --- | --- | --- |
| Agent attempt record отсутствует | Attempt не существует. Оставшиеся artifacts или временные файлы сами по себе его не создают. | Не восстанавливать attempt; такие файлы не участвуют в workflow. |
| Completion event отсутствует, как и подтверждённая Agent session activation | Attempt создан, но успешное выполнение не подтверждено durable. | Для Agent создать новую внутреннюю сессию, для Process повторно запустить тот же materialized command с тем же `n`. |
| Completion event отсутствует, но есть activations или оставшиеся после прерванной финализации файлы | Attempt не завершён. В частности, `/exit`, crash и возврат процесса без кандидата сами по себе не завершают Step и не создают durable-факт. | Только по явному `resume` продолжить последнюю activation в durable-порядке. Незавершённые файлы не передаются агенту и не участвуют в графе. |
| Agent attempt record оканчивается валидным `completed` и присутствуют все объявленные artifacts | Agent вернул управление с кандидатом completion либо Process вернул `0` с валидными outputs; attempt успешно завершён, а его artifacts зафиксированы. При пустом `outputs` их нет. | Не запускать executor; сделать artifacts доступными и пересчитать graph по `workflow.spec.md`. |
| Agent attempt record или artifact завершённого attempt не соответствует `format.spec.md` либо durable input activation противоречит `workflow.spec.md` | Run невозможно однозначно восстановить. | Завершить `resume` с ошибкой до запуска агента. |

Artifacts и completion публикуются только при возврате процесса агента с принятым кандидатом. Успешный ответ `attempt complete` разрешает агенту продолжить работу, заменить кандидат или активировать другую session и сам по себе не завершает Step. Crash до возврата процесса или до commit-точки финализации оставляет attempt незавершённым, а volatile-кандидат и возможные файловые остатки не сохраняют частичный результат агента и не участвуют в восстановлении.

Artifact без соответствующего Agent attempt record, файловый остаток attempt без
completion и временный файл атомарной записи не входят в модель и игнорируются;
их автоматическая очистка не входит в текущий scope.

## Fail-fast восстановление

После получения Run lock, но до запуска агента, вычисления frontier
или любой durable-записи orchestrator полностью проверяет materialized workflow,
все Agent attempt records и artifacts успешно завершённых attempts. Все файлы
должны соответствовать `format.spec.md` и не противоречить остальным фактам run;
соответствие workflow graph и durable input activation проверяется по
`workflow.spec.md`.

Любая ошибка этой проверки завершает `start` или `resume` до запуска агентов и
без изменения durable-модели run. Временные файлы атомарной записи, artifacts
без соответствующего attempt и файловые остатки attempt без completion моделью
не являются и на результат проверки не влияют.

После успешной проверки результат каждого attempt вычисляется только его Agent
type из materialized workflow и полного содержимого Agent attempt record;
отдельный сохранённый status не используется.

## Read-only inspection

- Read-only inspection загружает materialized workflow, attempts и artifacts через ту же validation границу, что `resume`.
- Storage boundary сравнивает fingerprint всех durable `spec.yaml`, `*.attempt.yaml` и `*.artifact` до и после validation, повторяет изменившийся snapshot не более четырёх раз, возвращает стабильную противоречивую модель как validation error и непрерывно меняющуюся модель как runtime error; `active.lock` и временные entries в fingerprint не входят.
- Последняя session вычисляется из полной durable-модели; производный status отдельно не записывается.
- Watch сразу публикует initial typed snapshot, затем с фиксированным интервалом 100 ms публикует только изменившиеся validated snapshots и завершается на `blocked`, `completed` или поддерживаемом termination signal.
- Verify с явно выбранным RunId проверяет только этот run.

## Source catalogs

- Source catalog commands перечисляют workflows, named Agents и prompt templates относительно выбранного state root без materialization, создания run или изменения файлов; каждый catalog целиком строится и проверяется до renderer, поэтому ошибка не даёт partial stdout.
- Workflow show выбирает ровно один source template, проверяет его структурную schema и symbolic IDs, сохраняет исходный порядок Steps и source references, но не читает config или prompts, не применяет defaults и не проверяет существование references либо graph reachability.
- Workflow plan использует ту же полную materialization и graph validation boundary, что `start`, но возвращает typed кандидат до резервирования RunId, Run lock и durable publication.

## Codex Agent type

- Type `codex` принимает непустой `model` и `reasoning` из множества `low`, `medium`, `high`, `xhigh`, `max`; executable по умолчанию — `codex`.
- Новая non-human session запускается как `codex exec --json --model <model> --config model_reasoning_effort="<reasoning>" <prompt>`, а native resume — как `codex exec resume --json --model <model> --config model_reasoning_effort="<reasoning>" <session-id> <prompt>`.
- Новая human session запускается как `codex --model <model> --config model_reasoning_effort="<reasoning>" <prompt>`, а native resume — как `codex resume --model <model> --config model_reasoning_effort="<reasoning>" <session-id> <prompt>`; stdout и stderr напрямую наследуют TTY, поэтому session activation доставляется agent-specific hook.
- Non-human stdout является JSONL event stream; первое событие `{"type":"thread.started","thread_id":"<session-id>"}` фиксирует native session activation, повтор того же ID идемпотентен, другой ID внутри одного process противоречив, а отсутствие события или невалидная JSON-строка являются adapter error.
- Неизвестные валидные JSONL events игнорируются для recovery; событие `item.completed` с `item.type = "agent_message"` и строковым `item.text` обновляет только volatile Agent session view.

## Claude Agent type

- Type `claude` принимает непустой `model` и `reasoning` из множества `low`, `medium`, `high`, `xhigh`, `max`; executable по умолчанию — `claude`.
- Новая non-human session запускается как `claude --print --output-format stream-json --verbose --model <model> --effort <reasoning> <prompt>`, а native resume добавляет `--resume <session-id>` перед prompt.
- Новая human session запускается как `claude --model <model> --effort <reasoning> <prompt>`, а native resume добавляет `--resume <session-id>` перед prompt; stdout и stderr напрямую наследуют TTY, поэтому session activation доставляется agent-specific hook.
- Non-human stdout является JSONL event stream; первое событие `{"type":"system","subtype":"init","session_id":"<session-id>"}` фиксирует native session activation, повтор того же ID идемпотентен, другой ID внутри одного process противоречив, а отсутствие init или невалидная JSON-строка являются adapter error.
- Неизвестные валидные JSONL events игнорируются для recovery; событие `assistant` с текстовыми content blocks обновляет только volatile Agent session view.

## Главный workflow

1. `orchestrator start` получает выбранный workflow, строит его полный materialized candidate в памяти и применяет те же проверки, что `validate`, не создавая данных run.
2. CLI резервирует свободный RunId по правилам разрешения коллизий, создаёт каталог run, захватывает exclusive Run lock и атомарно durable-публикует проверенный кандидат как `spec.yaml` до публикации первого Agent attempt.
3. `start` и последующие scheduling passes создают attempts только по правилам initial activation, dependencies, нумерации и durable inputs из `workflow.spec.md`; публикация attempt навсегда фиксирует его input.
4. Выбранные source attempts образуют input mapping, где artifact адресуется ключом `(source-step-id, input-id)`. Agent type получает сформированный UTF-8 prompt, mapping и control context и создаёт либо продолжает native session.
5. Пока процесс агента работает, переданные им source-файлы и принятый кандидат completion не входят в durable-модель run или workflow graph. Каждый валидный `attempt complete` передаёт полный набор artifacts, объявленный в `outputs` Step, и целиком заменяет предыдущий кандидат; после него агент может продолжать работу, передать другой набор или активировать другую session.
6. Когда процесс агента возвращает управление, supervisor сначала завершает обработку принятых control calls. При наличии кандидата он публикует artifacts и одной атомарной заменой record добавляет терминальный `completed`; с этого commit artifacts неизменяемы, attempt успешно завершён и graph пересчитывается по `workflow.spec.md`. Без кандидата Agent attempt record не изменяется, и attempt остаётся незавершённым.
7. Когда Process возвращает `0`, supervisor проверяет staging outputs и публикует artifacts с `completed`; ненулевой exit или невалидный output оставляет attempt незавершённым и не публикует частичный результат.
8. Возврат процесса supervisor наблюдает напрямую; hook и durable-событие для этого не используются. До user shutdown любой ненулевой exit code fail-fast завершает текущую команду, автоматический перезапуск attempt в этой команде не выполняется, Agent control context закрывается, Run lock остаётся у supervisor до завершения lifecycle, а производные статусы не записываются.
9. `orchestrator resume <run-id>` захватывает Run lock, проверяет durable-модель, находит незавершённые attempts и ready activations и запускает их по scheduling rules; Agent продолжает последнюю activation либо создаёт session, Process повторяет materialized command, а занятый lock завершает команду до запуска executors.
10. `resume` применяет таблицу восстановления ко всем опубликованным Agent attempt records; отдельная state-запись и обязательный output-маркер для этого не нужны.
