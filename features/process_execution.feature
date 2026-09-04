Feature: Исполнение произвольных процессов в workflow graph

  @format:workflow-template @workflow:inputs-и-prompt @cli:start
  Rule: Каждый Process arg является literal либо полным param, path или output placeholder и передаётся без shell
    Допустимые формы placeholders: {{param:<parameter-id>}}, {{path:<step-id>:<input-id>}} и {{output:<input-id>}}; content placeholder запрещён.

    @process
    Scenario: Process разрешает parameter, input path и output path отдельными аргументами
      Given подготовлен workflow с Process producer и Process consumer
      When запускается workflow с parameter mode содержащим пробел
      Then Process получает ровно шесть argv: --mode, fast mode, --input, absolute input path, --output, absolute output path
      And Process получает StepId convert, RunId, attempt 1, YAML input и output mappings без control endpoint
      And Process output опубликован как artifact
      And materialized workflow содержит точное значение parameter

    Scenario Outline: Невалидные run parameters отклоняются до создания run
      Given подготовлен workflow с обязательным parameter mode
      When start получает <parameters>
      Then команда завершается с кодом <code>
      And run не создан

      Examples:
        | parameters                      | code |
        | отсутствующий parameter         | 3    |
        | неизвестный parameter           | 3    |
        | повторяющийся parameter          | 2    |
        | parameter без разделителя equals | 2    |

  @format:workflow-template @workflow:validation
  Rule: Process schema и placeholders проверяются до создания run

    Scenario: Workflow tools сохраняют source Process и показывают materialized executor
      Given подготовлен source Process workflow с parameter mode
      When Process workflow читается и планируется через публичный API
      Then source Process сохраняет executable и argv
      And plan содержит absolute executable cwd и parameter declaration
      And run не создан

    Scenario Outline: Невалидный Process Step отклоняется preflight
      Given подготовлен Process Step с ошибкой <error>
      When workflow проверяется через публичный API
      Then validation отклоняет Process Step
      And run не создан

      Examples:
        | error                                                   |
        | одновременно указан Agent                               |
        | установлен human                                        |
        | content placeholder {{content:source:data}}              |
        | placeholder prefix-{{param:mode}} является частью argv   |
        | parameter placeholder {{param:missing}} неизвестен       |
        | path placeholder {{path:missing:data}} без dependency    |
        | path placeholder {{path:source:missing}} без output      |
        | output placeholder {{output:missing}} неизвестен         |

  @spec:process-executor @spec:native-resume @cli:коды-завершения
  Rule: Незавершённый Process attempt повторяется at-least-once

    @process
    Scenario: Ненулевой exit оставляет attempt для повторного resume
      Given подготовлен Process Step завершающийся успешно со второго запуска
      When start запускает Process первый раз
      Then команда завершается runtime error и attempt не завершён
      When run явно продолжается
      Then тот же Process attempt запущен второй раз
      And run завершён

  @spec:process-executor @format:artifacts
  Rule: Process completion проходит через durable artifact commit

    @process
    Scenario: Stdout Process публикуется как объявленный artifact
      Given подготовлен Process Step направляющий stdout в output
      When запускается workflow с Process Step
      Then stdout Process опубликован как artifact

    @process
    Scenario: Успешный Process без обязательного output не завершает attempt
      Given подготовлен Process Step не создающий объявленный output
      When запускается workflow с Process Step
      Then команда завершается runtime error и attempt не завершён
