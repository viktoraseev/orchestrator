Feature: Read-only inspection durable runs

  @spec:read-only-inspection @spec:сущности @format:корень-состояния-и-layout @cli:run-list-run-show-и-run-artifact
  Rule: Run list вычисляет состояние без побочных эффектов

    Scenario: Пустой корень даёт пустой список
      Given подготовлен пустой inspection root
      When выполняется run list через публичный API
      Then inspection завершается с кодом 0
      And inspection output пуст

    @process
    Scenario: Runs перечисляются по RunId с вычисленным состоянием
      Given подготовлены active, blocked и completed durable runs
      When запускается orchestrator run list
      Then inspection завершается с кодом 0
      And список runs отсортирован и содержит три вычисленных состояния
      And inspection не изменил durable state

    @process
    Scenario: Противоречивый run не даёт частичный stdout
      Given подготовлены completed и противоречивый durable runs
      When запускается orchestrator run list
      Then inspection завершается с кодом 3
      And inspection output пуст
      And inspection не изменил durable state

  @spec:read-only-inspection @workflow:initial-activation-dependencies-и-frontier @format:materialized-workflow @format:agent-attempt-record @cli:run-list-run-show-и-run-artifact
  Rule: Run show отображает validated read model

    Scenario: Active run показывает Steps attempts sessions и frontier
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

  @spec:read-only-inspection @format:artifact @format:идентификаторы-и-номера @cli:run-list-run-show-и-run-artifact
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
