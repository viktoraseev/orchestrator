Feature: Validation workflow
  Validate и preflight start полностью materialize и проверяют только выбранный workflow в памяти; validate не создаёт run, а start публикует snapshot лишь после успешного резервирования RunId.

  @cli:validate
  Rule: Явно выбранный workflow полностью проверяется в памяти
    Validation допускает cycles с bootstrap первого Step, outputs без consumers и graph без terminal Step; runtime использует уже validated materialized workflow и не пересчитывает статическую reachability.

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

  Rule: Структура workflow проверяется до ссылок и graph
    Source workflow является YAML mapping с optional `parameters` и обязательной непустой ordered sequence `steps`; parameters сопоставляет уникальные ParameterIds единственному type `string`, а каждый Step содержит только `id`, optional `agent`, optional `prompt`, optional `process`, обязательные `human`, `depends-on` и `outputs`.
    StepIds уникальны, `depends-on` и `outputs` являются sequences выражений all/one-of со symbolic IDs и могут быть пустыми; OutputIds уникальны во всём выражении; duplicate mapping keys, неизвестные поля и нарушения типов отклоняются до проверки ссылок.

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
        | parameters не mapping             | invalid type            |
        | неподдерживаемый type parameter   | неподдерживаемый type   |
        | steps не sequence                 | invalid type            |
        | повторяющийся root key            | duplicate field         |
        | синтаксически невалидный YAML      | невалидный workflow     |

  Rule: Каждый Step получает совместимого Agent с native resume
    Каждый Step получает явно названный Agent либо `default-agent`; validation проверяет существование Agent, его type, model, reasoning и поддержку native resume тем же preflight при validate и start, а materialized run повторно проверяется при resume без fallback на новую session.

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

  Rule: Prompt placeholders ссылаются только на input mapping Step
    Prompt template — произвольный UTF-8 Markdown без YAML-декодирования; `{{path:<step-id>:<input-id>}}` подставляет absolute artifact path, а `{{content:<step-id>:<input-id>}}` — точный UTF-8 content без дополнительного экранирования.
    Каждый PromptId существует; placeholders не содержат пробелы, первый описанный Step не содержит placeholders, а placeholder остальных Steps ссылается только на Step из `depends-on` и объявленный InputId из outputs этого source Step; неизвестный вид, неизвестная пара и незакрытый placeholder невалидны.

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

  Rule: Reachability учитывает bootstrap первого Step и полные dependency groups
    Первый Step статически достижим initial activation, следующий Step — когда достижима хотя бы одна согласованная альтернатива его dependencies; non-entry Step без dependencies и cycle без bootstrap-пути не имеют activation path и отклоняются.

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
