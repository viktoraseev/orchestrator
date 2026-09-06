Feature: Read-only inspection durable runs

  @format:корень-состояния-и-layout @format:artifact @cli:read-only-run-inspection
  Rule: Run inspection читает согласованный snapshot

    Read-only inspection загружает materialized workflow, attempts и artifacts через ту же validation boundary, что resume.
    Fingerprint включает durable spec.yaml, attempt records и artifacts, исключает active.lock и временные entries, повторяет изменившийся snapshot не более четырёх раз и различает стабильную validation error и непрерывные изменения с runtime error.

    Scenario: Стабильная противоречивая модель не становится typed snapshot
      Given подготовлены completed и противоречивый durable runs
      When строится typed inspection snapshot run 20 через публичный API
      Then inspection завершается с кодом 3
      And typed inspection snapshot отсутствует
      And inspection не изменил durable state

  @format:корень-состояния-и-layout @cli:read-only-run-inspection
  Rule: Run list вычисляет состояние без побочных эффектов

    Scenario: Пустой корень даёт пустой список
      Given подготовлен пустой inspection root
      When выполняется run list через публичный API
      Then inspection завершается с кодом 0
      And inspection output пуст
      And inspection не изменил durable state

    @process
    Scenario: Числовые runs перечисляются по RunId с вычисленным состоянием, а остальные entries игнорируются
      Given подготовлены active, blocked и completed durable runs и нечисловые entries
      When запускается orchestrator run list
      Then inspection завершается с кодом 0
      And text list содержит по одной отсортированной summary-строке для active, blocked и completed
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
      Given подготовлены active, blocked и completed durable runs
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

  @cli:read-only-run-inspection
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

  @format:artifact @format:идентификаторы-и-номера @cli:read-only-run-inspection
  Rule: Run artifact выбирает точную durable версию

    @process
    Scenario: Бинарный artifact копируется без преобразования
      Given подготовлен completed run с двумя версиями бинарного artifact
      When запускается orchestrator run artifact 30 0 result
      Then inspection завершается с кодом 0
      And stdout побайтово равен первой версии artifact
      And inspection не изменил durable state

    @process
    Scenario Outline: Неизвестный или неопубликованный artifact не даёт частичный stdout
      Given подготовлен completed run с двумя версиями бинарного artifact
      When запускается orchestrator run artifact 30 <attempt> <input>
      Then inspection завершается с кодом <code>
      And inspection output пуст

      Examples:
        | attempt | input   | code |
        | 99      | result  | 4    |
        | 0       | missing | 4    |
        | 1       | result  | 4    |

  @format:artifact @cli:read-only-run-inspection
  Rule: Run artifacts перечисляет только опубликованные версии

    Scenario: Typed API возвращает artifact descriptors
      Given подготовлен completed run с двумя версиями бинарного artifact
      When строится typed inspection snapshot run 30 через публичный API
      Then inspection завершается с кодом 0
      And typed snapshot содержит две версии result
      And inspection не изменил durable state

    @process
    Scenario: Completed artifacts перечисляются в deterministic order
      Given подготовлен completed run с двумя версиями бинарного artifact
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

  @cli:read-only-run-inspection
  Rule: Inspection renderers используют одну typed model

    @process
    Scenario: JSON show сохраняет числовые и nullable поля
      Given подготовлен active durable run 10 без session
      When запускается orchestrator run show 10 в JSON
      Then inspection завершается с кодом 0
      And JSON show содержит snake_case typed snapshot run 10 с number, null и arrays

    @process
    Scenario: Text является default renderer списка
      Given подготовлены active, blocked и completed durable runs
      When запускается orchestrator run list для completed workflow
      Then inspection завершается с кодом 0
      And inspection output содержит только completed run

  @cli:read-only-run-inspection
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
    Scenario: Active watch игнорирует volatile entries и публикует durable completion
      Given подготовлен active durable run 10 без session
      When watch наблюдает изменения lock и временного entry до durable completion
      Then inspection завершается с кодом 0
      And watch опубликовал initial active и final completed snapshots
      And watch не публиковал snapshot для volatile изменений за три polling interval

    @process
    Scenario: SIGTERM завершает active watch общим signal exit code
      Given подготовлен active durable run 10 без session
      When active watch получает SIGTERM после initial snapshot
      Then inspection завершается с кодом 143
      And watch опубликовал только initial active snapshot
      And inspection не изменил durable state

  @cli:read-only-run-inspection
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
