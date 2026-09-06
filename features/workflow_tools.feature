Feature: Read-only workflow tooling

  @cli:validate
  Rule: Bulk validate возвращает полный отчёт по source workflow catalog

    Scenario: Публичный API сохраняет valid и invalid результаты в сортированном отчёте
      Given подготовлен workflow catalog с valid beta и invalid alpha
      When все workflows проверяются через публичный API
      Then workflow tool завершается с кодом 3
      And typed bulk report равен invalid alpha с diagnostics и valid beta без diagnostics по порядку
      And каталог run отсутствует после workflow tool

    @process
    Scenario: Bulk validate JSON публикуется полностью перед кодом 3
      Given подготовлен workflow catalog с valid beta и invalid alpha
      When запускается orchestrator validate --all в JSON
      Then workflow tool завершается с кодом 3
      And JSON bulk report равен invalid alpha и valid beta по порядку

    @process
    Scenario: Bulk validate text публикуется полностью перед кодом 3
      Given подготовлен workflow catalog с valid beta и invalid alpha
      When запускается orchestrator validate --all
      Then workflow tool завершается с кодом 3
      And bulk report text содержит invalid alpha перед valid beta

    @process
    Scenario: Пустой workflow catalog даёт успешный пустой отчёт
      Given подготовлен пустой workflow tool root
      When запускается orchestrator validate --all
      Then workflow tool завершается с кодом 0
      And workflow tool stdout пуст

    @process
    Scenario: --all и WorkflowId взаимоисключаемы до чтения state root
      Given подготовлен пустой workflow tool root
      When запускается orchestrator validate delivery --all
      Then workflow tool завершается с кодом 2
      And workflow tool stdout пуст

    @process
    Scenario: Невалидный WorkflowId contract file не даёт partial bulk report
      Given подготовлен workflow tool contract file с невалидным ID
      When запускается orchestrator validate --all
      Then workflow tool завершается с кодом 3
      And workflow tool stdout пуст

  @cli:source-catalogs
  Rule: Workflow graph проверяет только source topology

    Scenario: Typed graph сохраняет bootstrap, nodes и dependency edges
      Given подготовлен source graph delivery без config и prompts
      When graph delivery строится через публичный API
      Then workflow tool завершается с кодом 0
      And typed graph равен workflow delivery, bootstrap plan, nodes plan и implement, edge plan to implement
      And каталог run отсутствует после workflow tool

    @process
    Scenario: Workflow graph text имеет стабильный source order
      Given подготовлен source graph delivery без config и prompts
      When запускается orchestrator workflow graph delivery
      Then workflow tool завершается с кодом 0
      And graph text состоит из absolute path workflow delivery, bootstrap plan и edge plan to implement

    @process
    Scenario: Workflow graph JSON возвращает typed object
      Given подготовлен source graph delivery без config и prompts
      When запускается orchestrator workflow graph delivery в JSON
      Then workflow tool завершается с кодом 0
      And JSON graph равен workflow delivery, absolute path, bootstrap plan, nodes plan и implement, edge plan to implement

    @process
    Scenario: Статически недостижимый source graph не даёт stdout
      Given подготовлен недостижимый source graph delivery
      When запускается orchestrator workflow graph delivery
      Then workflow tool завершается с кодом 3
      And workflow tool stdout пуст

    @process
    Scenario: Неизвестная dependency source graph не даёт stdout
      Given подготовлен source graph delivery с неизвестной dependency
      When запускается orchestrator workflow graph delivery
      Then workflow tool завершается с кодом 3
      And workflow tool stdout пуст

  @cli:source-catalogs @cli:start
  Rule: Workflow plan показывает тот же кандидат, который подготовил бы start
    Validate, workflow plan и start используют общую полную materialization и graph validation boundary до создания данных run; plan возвращает typed кандидат до резервирования RunId, Run lock и durable publication.
    JSON plan содержит ровно snake_case поля `workflow_id`, `max_parallel_agents`, `parameters` и `steps`; parameters перечисляет ParameterIds без runtime values, каждый Step содержит `id`, nullable `agent`, nullable `prompt`, nullable `process`, `human`, `depends_on` и `outputs`, а Agent object — только `type`, `model` и `reasoning`.

    Scenario: Typed plan содержит effective Agent, prompt и parallel limit
      Given подготовлен полностью materializable workflow delivery
      When plan delivery строится через публичный API
      Then workflow tool завершается с кодом 0
      And typed plan содержит effective Agent и prompt content
      And каталог run отсутствует после workflow tool

    @process
    Scenario: Workflow plan JSON возвращает materialized candidate
      Given подготовлен полностью materializable workflow delivery
      When запускается orchestrator workflow plan delivery в JSON
      Then workflow tool завершается с кодом 0
      And JSON plan содержит effective limit Agent и prompt
      And JSON plan имеет закрытую Agent schema и только declarations parameters

    @process
    Scenario: Workflow plan text не раскрывает prompt content
      Given подготовлен полностью materializable workflow delivery
      When запускается orchestrator workflow plan delivery
      Then workflow tool завершается с кодом 0
      And plan text содержит effective summaries без prompt content

    @process
    Scenario: Невалидный prompt plan не создаёт run и не даёт stdout
      Given подготовлен workflow delivery с non-UTF-8 prompt
      When запускается orchestrator workflow plan delivery
      Then workflow tool завершается с кодом 3
      And workflow tool stdout пуст
      And каталог run отсутствует после workflow tool
