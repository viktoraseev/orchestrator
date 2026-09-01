Feature: Read-only catalogs source definitions

  @spec:source-catalogs @format:корень-состояния-и-layout @format:идентификаторы-и-номера @cli:source-catalogs
  Rule: Workflow catalog перечисляет source templates без materialization

    Scenario: Пустой workflow catalog успешен через публичный API
      Given подготовлен пустой source catalog root
      When workflow catalog строится через публичный API
      Then catalog завершается с кодом 0
      And catalog output пуст

    @process
    Scenario: Workflow templates сортируются и имеют абсолютные paths в JSON
      Given подготовлены workflow templates beta и alpha с посторонними entries
      When запускается orchestrator workflow list в JSON
      Then catalog завершается с кодом 0
      And JSON workflow catalog содержит alpha и beta по порядку
      And catalog не изменил source state

    @process
    Scenario: Невалидный WorkflowId contract file не даёт partial stdout
      Given подготовлен workflow contract file с невалидным ID
      When запускается orchestrator workflow list
      Then catalog завершается с кодом 3
      And catalog output пуст

  @spec:source-catalogs @format:config-yaml @cli:source-catalogs
  Rule: Agent catalog использует полную config validation

    Scenario: Typed Agent catalog сохраняет параметры named Agents
      Given подготовлен config с Agents beta и alpha
      When Agent catalog строится через публичный API
      Then catalog завершается с кодом 0
      And typed Agent catalog содержит alpha и beta по порядку

    @process
    Scenario: Agent text renderer детерминирован
      Given подготовлен config с Agents beta и alpha
      When запускается orchestrator agent list
      Then catalog завершается с кодом 0
      And Agent catalog text содержит обе validated записи по порядку
      And catalog не изменил source state

    @process
    Scenario: Невалидный скрытый config field не даёт partial stdout
      Given подготовлен невалидный config для Agent catalog
      When запускается orchestrator agent list в JSON
      Then catalog завершается с кодом 3
      And catalog output пуст

  @spec:source-catalogs @format:prompt-template @format:идентификаторы-и-номера @cli:source-catalogs
  Rule: Prompt catalog полностью читает UTF-8 templates

    Scenario: Typed prompt catalog считает UTF-8 bytes
      Given подготовлен prompt template unicode
      When prompt catalog строится через публичный API
      Then catalog завершается с кодом 0
      And typed prompt unicode имеет документированный размер bytes

    @process
    Scenario: Prompt JSON catalog сортируется и игнорирует temporary entries
      Given подготовлены prompt templates beta и alpha с temporary entry
      When запускается orchestrator prompt list в JSON
      Then catalog завершается с кодом 0
      And JSON prompt catalog содержит alpha и beta по порядку
      And catalog не изменил source state

    @process
    Scenario: Non-UTF-8 prompt не даёт partial stdout
      Given подготовлен non-UTF-8 prompt template
      When запускается orchestrator prompt list
      Then catalog завершается с кодом 3
      And catalog output пуст
