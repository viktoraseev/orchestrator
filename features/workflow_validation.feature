Feature: Validation workflow
  Выбранный workflow полностью materialize и проверяется в памяти без создания run.

  @cli:validate @workflow:validation @format:workflow-template @format:prompt-template
  Rule: Явно выбранный workflow полностью проверяется в памяти

    Scenario Outline: Допустимые формы workflow проходят validation
      Given подготовлен кандидат workflow "<candidate>"
      When workflow delivery проверяется через публичный API
      Then validation завершается с кодом 0
      And результат validation равен "workflow delivery: valid"
      And каталог run отсутствует

      Examples:
        | candidate                                      |
        | линейный graph с обоими видами placeholders    |
        | entry cycle без terminal Step                  |
        | output без consumers                           |
        | выбранный workflow с невалидными невыбранными файлами |

  @workflow:validation @format:workflow-template
  Rule: Структура workflow проверяется до ссылок и graph

    Scenario Outline: Невалидная структура workflow отклоняется
      Given подготовлен кандидат workflow "<candidate>"
      When workflow delivery проверяется через публичный API
      Then validation завершается с кодом 3
      And diagnostics содержит "<diagnostic>"

      Examples:
        | candidate                         | diagnostic             |
        | пустой steps                      | steps должен быть непустым |
        | отсутствующее обязательное поле   | missing field           |
        | неизвестное поле Step             | unknown field           |
        | повторяющийся StepId              | StepId повторяется      |
        | повторяющийся depends-on          | depends-on содержит повтор |
        | повторяющийся output              | outputs содержит повтор |
        | невалидный StepId                 | не соответствует kebab-case |
        | невалидный ParameterId с точкой   | не соответствует kebab-case |
        | невалидный InputId с точкой       | не соответствует kebab-case |
        | невалидный PromptId с точкой      | не соответствует kebab-case |
        | синтаксически невалидный YAML      | невалидный workflow     |

  @workflow:validation @format:config-yaml
  Rule: Каждый Step получает совместимого Agent с native resume
    Agent type без поддержки native resume отклоняется тем же preflight при validate и start; материализованный run повторно проверяется при resume без fallback на новую session.

    Scenario Outline: Невалидный выбор Agent отклоняется
      Given подготовлен кандидат workflow "<candidate>"
      When workflow delivery проверяется через публичный API
      Then validation завершается с кодом 3
      And diagnostics содержит "<diagnostic>"

      Examples:
        | candidate                         | diagnostic          |
        | Agent не указан без default-agent | Agent не указан     |
        | явный Agent отсутствует           | отсутствует в config |
        | невалидный AgentId                | не соответствует kebab-case |
        | Agent type без native resume      | native resume       |
        | невалидный config                 | невалидный config   |

  @workflow:validation @format:prompt-template
  Rule: Prompt placeholders ссылаются только на input mapping Step

    Scenario Outline: Невалидный prompt отклоняется
      Given подготовлен кандидат workflow "<candidate>"
      When workflow delivery проверяется через публичный API
      Then validation завершается с кодом 3
      And diagnostics содержит "<diagnostic>"

      Examples:
        | candidate                         | diagnostic              |
        | отсутствующий Prompt              | не существует           |
        | placeholder первого Step          | первого Step            |
        | неизвестный вид placeholder       | неизвестный вид          |
        | пробел внутри placeholder          | невалидный placeholder   |
        | незакрытый placeholder             | незакрытый placeholder   |
        | placeholder вне depends-on         | вне depends-on           |
        | placeholder неизвестного output    | неизвестный output       |
        | Prompt не UTF-8                    | не является UTF-8        |
        | Prompt не regular file             | не является regular file |

  @workflow:validation
  Rule: Reachability учитывает bootstrap первого Step и полные dependency groups

    Scenario Outline: Недостижимый graph отклоняется
      Given подготовлен кандидат workflow "<candidate>"
      When workflow delivery проверяется через публичный API
      Then validation завершается с кодом 3
      And diagnostics содержит "<diagnostic>"

      Examples:
        | candidate                         | diagnostic          |
        | неизвестный dependency            | неизвестный Step    |
        | non-entry Step без dependencies    | статически недостижим |
        | cycle без bootstrap-пути           | статически недостижим |

    Scenario: Структурная ошибка предшествует ошибке ссылки более позднего Step
      Given подготовлен кандидат workflow "структурная и ссылочная ошибки"
      When workflow delivery проверяется через публичный API
      Then validation завершается с кодом 3
      And diagnostics содержит "outputs содержит повтор"
      And diagnostics не содержит "неизвестный Step"

  @cli:выбор-workflow @cli:validate
  Rule: Workflow по умолчанию использует тот же validate entrypoint

    Scenario: Валидный workflow выбирается из config
      Given подготовлен кандидат workflow "default-workflow delivery"
      When workflow по умолчанию проверяется через публичный API
      Then validation завершается с кодом 0
      And результат validation равен "workflow delivery: valid"

    Scenario: Отсутствующий default-workflow требует явного выбора
      Given подготовлен кандидат workflow "линейный graph с обоими видами placeholders"
      When workflow по умолчанию проверяется через публичный API
      Then validation завершается с кодом 2
      And diagnostics содержит "передайте WorkflowId или настройте default-workflow"

    Scenario: Отсутствующий template из default-workflow не имеет fallback
      Given подготовлен кандидат workflow "default-workflow missing"
      When workflow по умолчанию проверяется через публичный API
      Then validation завершается с кодом 4
      And diagnostics содержит "workflow 'missing' не существует"

    Scenario: Невалидный config отклоняется до чтения default workflow
      Given подготовлен кандидат workflow "невалидный config с существующим workflow"
      When workflow по умолчанию проверяется через публичный API
      Then validation завершается с кодом 3
      And diagnostics содержит "невалидный config"

    Scenario: Явная и default-формы возвращают одну validation error
      Given подготовлен кандидат workflow "default-workflow с недостижимым Step"
      When workflow delivery проверяется обеими формами публичного API
      Then обе формы validation возвращают одинаковый результат
