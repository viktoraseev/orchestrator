Feature: Read-only inspection durable runs

  Rule: Run inspection читает согласованный snapshot

    Read-only inspection загружает materialized workflow, attempts и artifacts через ту же validation boundary, что resume.
    Fingerprint включает durable spec.yaml, attempt records и artifacts, исключает active.lock и временные entries, повторяет изменившийся snapshot не более четырёх раз и различает стабильную validation error и непрерывные изменения с runtime error.

    Scenario: Стабильная противоречивая модель не становится typed snapshot
      Given подготовлены completed и противоречивый durable runs
      When строится typed inspection snapshot run 20 через публичный API
      Then inspection завершается с кодом 3
      And typed inspection snapshot отсутствует
      And inspection не изменил durable state

    Scenario Outline: Каждая inspection-команда принимает snapshot, стабилизировавшийся на четвёртой попытке чтения
      Given подготовлен completed run 30 с изменениями после первых 3 fingerprint
      When выполняется "<command>" через публичный command entrypoint
      Then inspection завершается с кодом 0
      And inspection output непуст

      Examples:
        | command       |
        | run list      |
        | run show      |
        | run artifacts |
        | run artifact  |
        | run verify    |

    Scenario Outline: Каждая inspection-команда исчерпывает лимит при изменении snapshot на четвёртой попытке чтения
      Given подготовлен completed run 30 с изменениями после первых 4 fingerprint
      When выполняется "<command>" через публичный command entrypoint
      Then inspection завершается с кодом 1
      And diagnostics сообщает о непрерывно меняющемся snapshot после четырёх попыток
      And inspection output пуст

      Examples:
        | command       |
        | run list      |
        | run show      |
        | run artifacts |
        | run artifact  |
        | run verify    |

  Rule: Run list вычисляет состояние без побочных эффектов

    Scenario: Пустой корень даёт пустой список
      Given подготовлен пустой inspection root
      When выполняется run list через публичный API
      Then inspection завершается с кодом 0
      And inspection output пуст
      And inspection не изменил durable state

    @process
    Scenario: Числовые runs перечисляются по RunId с вычисленным состоянием, а остальные entries игнорируются
      Given подготовлены active, закрытый условный и completed durable runs и нечисловые entries
      When запускается orchestrator run list
      Then inspection завершается с кодом 0
      And text list содержит по одной отсортированной summary-строке для active, закрытого условного и completed
      And inspection не изменил durable state

    @process
    Scenario: Противоречивый run не даёт частичный stdout
      Given подготовлены completed и противоречивый durable runs
      When запускается orchestrator run list
      Then inspection завершается с кодом 3
      And inspection output пуст
      And inspection не изменил durable state

    @process
    Scenario: Скрытый фильтром противоречивый run всё равно отклоняет полный список
      Given подготовлены completed и противоречивый durable runs
      When запускается orchestrator run list только для completed
      Then inspection завершается с кодом 3
      And inspection output пуст
      And inspection не изменил durable state

    @process
    Scenario: State filters объединяются как OR, workflow как AND, а повторы идемпотентны
      Given подготовлены active, закрытый условный и completed durable runs
      When запускается orchestrator run list с active, completed, completed и workflow completed
      Then inspection завершается с кодом 0
      And inspection output содержит только completed run

    @process
    Scenario Outline: Невалидный list filter отклоняется до чтения runs
      Given подготовлены completed и противоречивый durable runs
      When запускается orchestrator run list с невалидным filter "<filter>"
      Then inspection завершается с кодом 2
      And inspection output пуст

      Examples:
        | filter      |
        | state       |
        | format      |
        | workflow-id |

  Rule: Run show отображает validated read model

    Последняя session и status attempts и run вычисляются из полной durable-модели и отдельно не сохраняются.

    Scenario: Active run вычисляет status и показывает последнюю session, Steps, attempts и frontier
      Given подготовлен active durable run с session и ready Step
      When выполняется run show 20 через публичный API
      Then inspection завершается с кодом 0
      And run show содержит workflow Steps attempt session и ready frontier
      And inspection не изменил durable state

    @process
    Scenario: Занятый run доступен для read-only inspection
      Given подготовлен active durable run с удерживаемым lock
      When запускается orchestrator run show 20
      Then inspection завершается с кодом 0
      And run show содержит active run 20

    @process
    Scenario: Неизвестный run возвращает 4 без stdout
      Given подготовлен пустой inspection root
      When запускается orchestrator run show 404
      Then inspection завершается с кодом 4
      And inspection output пуст

  Rule: Run artifact выбирает точную durable версию
    Выбранный artifact доступен только у completed attempt и отдаётся как точные bytes regular durable-файла с ключом `(attempt-n, step-id, input-id)` без текстового преобразования.

    @process
    Scenario: Бинарный artifact копируется без преобразования
      Given подготовлен run с двумя завершёнными версиями бинарного artifact
      When запускается orchestrator run artifact 30 0 result
      Then inspection завершается с кодом 0
      And stdout побайтово равен первой версии artifact
      And inspection не изменил durable state

    @process
    Scenario Outline: Неизвестный или неопубликованный artifact не даёт частичный stdout
      Given подготовлен run с двумя завершёнными версиями бинарного artifact
      When запускается orchestrator run artifact 30 <attempt> <input>
      Then inspection завершается с кодом <code>
      And inspection output пуст

      Examples:
        | attempt | input   | code |
        | 99      | result  | 4    |
        | 0       | missing | 4    |
        | 3       | result  | 4    |

  Rule: Run artifacts перечисляет только опубликованные версии

    Scenario: Typed API возвращает artifact descriptors
      Given подготовлен run с двумя завершёнными версиями бинарного artifact
      When строится typed inspection snapshot run 30 через публичный API
      Then inspection завершается с кодом 0
      And typed snapshot содержит две версии result
      And inspection не изменил durable state

    @process
    Scenario: Completed artifacts перечисляются в deterministic order
      Given подготовлен run с двумя завершёнными версиями бинарного artifact
      When запускается orchestrator run artifacts 30
      Then inspection завершается с кодом 0
      And список artifacts содержит две опубликованные версии
      And inspection не изменил durable state

    @process
    Scenario: Пустой набор completed artifacts успешен
      Given подготовлен completed durable run 30
      When запускается orchestrator run artifacts 30
      Then inspection завершается с кодом 0
      And inspection output пуст

  Rule: Inspection renderers используют одну typed model

    @process
    Scenario: JSON show сохраняет числовые и nullable поля
      Given подготовлен active durable run 10 без session
      When запускается orchestrator run show 10 в JSON
      Then inspection завершается с кодом 0
      And JSON show содержит snake_case typed snapshot run 10 с number, null и arrays

    @process
    Scenario: Text является default renderer списка
      Given подготовлены active, закрытый условный и completed durable runs
      When запускается orchestrator run list для completed workflow
      Then inspection завершается с кодом 0
      And inspection output содержит только completed run

  Rule: Inspection CLI проверяет selectors и полную durable-модель до stdout

    @process
    Scenario Outline: Ошибка selector или durable-модели не даёт partial stdout
      Given подготовлены valid artifact run и противоречивый run
      When запускается inspection-команда для случая "<case>"
      Then inspection завершается с кодом <code>
      And inspection output пуст

      Examples:
        | case                         | code |
        | невалидный RunId             | 2    |
        | невалидный attempt number    | 2    |
        | невалидный InputId           | 2    |
        | неизвестный RunId            | 4    |
        | неизвестный attempt          | 4    |
        | неизвестный InputId          | 4    |
        | artifact незавершённого attempt | 4    |
        | show противоречивого run     | 3    |
        | artifacts противоречивого run | 3    |
        | artifact противоречивого run | 3    |

    @process
    Scenario: Non-regular artifact делает durable-модель противоречивой
      Given подготовлен completed run с non-regular artifact
      When запускается orchestrator run artifact 30 0 result
      Then inspection завершается с кодом 3
      And inspection output пуст

  Rule: Run watch публикует только изменившиеся snapshots

    Watch немедленно публикует initial typed snapshot, затем с фиксированным интервалом 100 ms публикует только изменившиеся validated snapshots и завершается на blocked, completed или поддерживаемом termination signal.

    @process
    Scenario: Terminal initial snapshot завершает watch
      Given подготовлен completed durable run 30
      When запускается orchestrator run watch 30 в JSON
      Then inspection завершается с кодом 0
      And watch опубликовал один JSON snapshot
      And inspection не изменил durable state

    @process
    Scenario: Terminal blocked snapshot завершает watch
      Given подготовлен durable cycle run 20 с выбранной внешней частью входа и отсутствующим feedback
      When запускается orchestrator run watch 20 в JSON
      Then inspection завершается с кодом 0
      And watch опубликовал один blocked JSON snapshot
      And inspection не изменил durable state

    @process
    Scenario: Text watch немедленно публикует initial snapshot как show document
      Given подготовлен completed durable run 30
      When запускается text watch 30 с ожиданием немедленного initial snapshot
      Then inspection завершается с кодом 0
      And initial snapshot опубликован менее чем за 75 ms от запуска процесса
      And watch опубликовал один text show document
      And inspection не изменил durable state

    @process
    Scenario: Text watch публикует полные show documents каждые 100 ms
      Given подготовлен active durable run 10 без session
      When text watch синхронизирован active snapshot и наблюдает три изменения и completion
      Then inspection завершается с кодом 0
      And четыре следующих snapshot опубликованы не быстрее чем за 360 ms и менее чем за 500 ms
      And text watch опубликовал подряд полные initial, четыре изменённых active и final completed show documents

    @process
    Scenario: Active watch игнорирует volatile entries и публикует durable completion
      Given подготовлен active durable run 10 без session
      When watch наблюдает изменения lock и временного entry до durable completion
      Then inspection завершается с кодом 0
      And watch опубликовал initial active и final completed snapshots
      And watch не публиковал snapshot для volatile изменений за три polling interval

    @process
    Scenario: Ошибка consistent snapshot не публикует частичный новый watch snapshot
      Given подготовлен active run с outputs для меняющегося artifact
      When watch наблюдает completion с непрерывно меняющимся durable artifact
      Then inspection завершается с кодом 1
      And diagnostics сообщает о непрерывно меняющемся snapshot после четырёх попыток
      And watch опубликовал только initial active snapshot

    @process
    Scenario Outline: Termination signal завершает active watch общим signal exit code
      Given подготовлен active durable run 10 без session
      When active watch получает <signal> после initial snapshot
      Then inspection завершается с кодом <code>
      And watch опубликовал только initial active snapshot
      And inspection не изменил durable state

      Examples:
        | signal  | code |
        | SIGHUP  | 129  |
        | SIGINT  | 130  |
        | SIGTERM | 143  |

  Rule: Run verify формирует полный отчёт до exit code

    Verify с явно выбранным RunId проверяет только этот run.

    @process
    Scenario: Verify продолжает после invalid run
      Given подготовлены valid и два противоречивых durable runs
      When запускается orchestrator run verify в JSON
      Then inspection завершается с кодом 3
      And verify JSON report содержит все три runs по RunId и diagnostics каждого invalid run
      And inspection не изменил durable state

    @process
    Scenario: Явный RunId изолирует verify от противоречивых соседних runs
      Given подготовлены valid и два противоречивых durable runs
      When запускается orchestrator run verify 10 в JSON
      Then inspection завершается с кодом 0
      And verify JSON report содержит только valid run 10
      And inspection не изменил durable state

    @process
    Scenario: Text verify перечисляет valid и invalid runs до итогового кода
      Given подготовлены completed и противоречивый durable runs
      When запускается orchestrator run verify в text
      Then inspection завершается с кодом 3
      And verify text report содержит valid run 10 перед invalid run 20 с непустой diagnostic
      And inspection не изменил durable state

    @process
    Scenario: Неизвестный явно выбранный run verify возвращает 4 без stdout
      Given подготовлен пустой inspection root
      When запускается orchestrator run verify 404 в text
      Then inspection завершается с кодом 4
      And inspection output пуст

    @process
    Scenario: Global I/O error прерывает verify без partial stdout
      Given inspection run catalog недоступен как directory
      When запускается orchestrator run verify в text
      Then inspection завершается с кодом 1
      And inspection output пуст
