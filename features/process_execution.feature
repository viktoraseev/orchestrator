Feature: Исполнение произвольных процессов в workflow graph
  Process executor — materialized executable, cwd, argv и optional stdout output обычного non-human Step; он не является Agent type, не имеет native session или control context и завершает attempt только через проверенный exit и supervisor-owned artifact commit.

  @format:workflow-template @cli:start @cli:сигналы-и-закрытие-терминала @cli:вывод-команд
  Rule: Process получает materialized argv, environment и non-interactive process boundary
    Step executor является ровно одним Agent или Process; Process запускает materialized executable с argv, не является Agent type и не участвует в native session protocol.
    Объявленный строковый run parameter проверяется до резервирования run, сохраняется в materialized workflow и имеет одно неизменное durable значение в argv каждого Process attempt.
    Допустимые формы placeholders: {{param:<parameter-id>}}, {{path:<step-id>:<input-id>}} и {{output:<input-id>}}; content placeholder запрещён.
    Перед запуском Process supervisor создаёт уникальный staging regular-file path для каждого объявленного output, передаёт mapping в ORC_OUTPUT и разрешает {{output:…}} в argv в соответствующий path; Process может создать или заменить этот файл, но не пишет durable artifact напрямую.
    Process наследует environment supervisor с заменой ORC_STEP_ID, ORC_RUN_ID, ORC_ATTEMPT, ORC_INPUT и ORC_OUTPUT, запускается отдельной process group с stdin: null, наследуемым stderr и stdout согласно stdout; stdout: <input-id> направляет точные bytes stdout в staging path этого output, а без stdout stdout наследуется.

    @process
    Scenario: Process разрешает parameter, input path и output path отдельными аргументами
      Given подготовлен workflow с Process producer и Process consumer
      When запускается workflow с parameter mode содержащим пробел
      Then Process получает ровно шесть argv: --mode, fast mode, --input, absolute input path, --output, absolute output path
      And Process получает StepId convert, RunId, attempt 1, YAML input и output mappings без control endpoint
      And Process output опубликован как artifact
      And materialized workflow содержит точное значение parameter

    @process
    Scenario: Process не читает stdin supervisor и наследует stdout и stderr
      Given подготовлен Process Step проверяющий non-interactive streams
      When workflow с Process Step запускается с данными в stdin supervisor
      Then Process завершился без stdin и его stdout и stderr наблюдаемы

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

  @cli:resume @cli:коды-завершения @format:agent-attempt-record
  Rule: Незавершённый Process attempt повторяется at-least-once
    Process не имеет native session и не создаёт session activation; явный resume повторно запускает тот же unfinished attempt с materialized executable, cwd, parameters и аргументами из durable input mapping, поэтому внешние side effects обязаны быть идемпотентными.
    Ненулевой exit code Process является runtime failure, не сохраняется в attempt record, не публикует staging outputs, не добавляет terminal event и не запускает новые attempts или автоматический restart; код процесса не становится exit code orchestrator.

    @process
    Scenario: Ненулевой exit оставляет attempt для повторного resume
      Given подготовлен Process Step завершающийся успешно со второго запуска
      When start запускает Process первый раз
      Then команда завершается runtime error и attempt не завершён
      And exit code Process не записан и автоматический restart не выполнен
      When run явно продолжается
      Then тот же Process attempt запущен второй раз
      And оба запуска получили одинаковые executable, cwd, parameter и durable input mapping
      And Process attempt не содержит session activation
      And run завершён

  @format:artifacts @cli:коды-завершения
  Rule: Process completion проходит через durable artifact commit
    После exit code 0 supervisor проверяет, что каждый объявленный output существует и является regular file, полностью читает staging files и публикует artifacts и completed через ту же commit-точку, что Agent completion; отсутствующий или невалидный output является runtime failure без terminal event.

    @process
    Scenario: Stdout Process публикуется как объявленный artifact
      Given подготовлен Process Step направляющий stdout в output
      When запускается workflow с Process Step
      Then stdout Process опубликован как artifact

    @process
    Scenario: Успешный Process без обязательного output не завершает attempt
      Given подготовлен Process Step создающий только один из двух объявленных outputs
      When запускается workflow с Process Step
      Then команда завершается runtime error и attempt не завершён
      And Process artifacts не опубликованы

    @process
    Scenario: Успешный Process с non-regular output не завершает attempt
      Given подготовлен Process Step создающий directory вместо output
      When запускается workflow с Process Step
      Then команда завершается runtime error и attempt не завершён
      And Process artifacts не опубликованы
