Feature: CLI validate
  CLI отображает результат того же публичного validate entrypoint.

  @cli:validate @cli:выбор-workflow @process
  Rule: Явный WorkflowId имеет стабильный CLI-контракт

    Scenario: Валидный явно выбранный workflow
      Given подготовлен кандидат workflow "выбранный workflow с невалидными невыбранными файлами"
      When запускается orchestrator validate delivery
      Then validation завершается с кодом 0
      And результат validation равен "workflow delivery: valid"

    Scenario: Невалидный явный WorkflowId
      Given подготовлен кандидат workflow "линейный graph с обоими видами placeholders"
      When запускается orchestrator validate Bad-ID
      Then validation завершается с кодом 2
      And diagnostics начинается с "error:"

    Scenario: Отсутствующий явно выбранный workflow
      Given подготовлен корень без workflow missing
      When запускается orchestrator validate missing
      Then validation завершается с кодом 4
      And diagnostics начинается с "error: validate:"

    Scenario: Больше одного WorkflowId отклоняется CLI parser
      Given подготовлен кандидат workflow "линейный graph с обоими видами placeholders"
      When запускается orchestrator validate delivery extra
      Then validation завершается с кодом 2
      And diagnostics начинается с "error:"

  @cli:validate @cli:выбор-workflow @process
  Rule: Validate без аргумента выбирает только default-workflow

    Scenario: Валидный workflow по умолчанию
      Given подготовлен кандидат workflow "default-workflow delivery"
      When запускается orchestrator validate без аргумента
      Then validation завершается с кодом 0
      And результат validation равен "workflow delivery: valid"

    Scenario: Default-workflow не настроен
      Given подготовлен кандидат workflow "линейный graph с обоими видами placeholders"
      When запускается orchestrator validate без аргумента
      Then validation завершается с кодом 2
      And diagnostics начинается с "error: validate:"

    Scenario: Default-workflow ссылается на отсутствующий template
      Given подготовлен кандидат workflow "default-workflow missing"
      When запускается orchestrator validate без аргумента
      Then validation завершается с кодом 4
      And diagnostics начинается с "error: validate:"
