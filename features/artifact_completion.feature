Feature: Публикация Agent completion и artifacts
  Artifact — версионируемый результат с durable-ключом `(attempt-n, step-id, input-id)`; source-файлы остаются во владении Agent, а workflow видит только полный неизменяемый набор последнего принятого completion после возврата Agent.

  @cli:session-activate-и-attempt-complete @format:artifact @format:agent-attempt-record
  Rule: Attempt complete принимает полный snapshot объявленных outputs
    Каждый вызов передаёт по одному absolute path на доступный regular file для каждого output либо пустой набор для Step без outputs; source может находиться вне run и быть symbolic link, а parent синхронно копирует его bytes в volatile-кандидат без переноса source-файла.

    Scenario: Step без outputs принимает пустой completion
      Given подготовлен single-step workflow без outputs
      When Agent передаёт completion без artifacts и возвращает управление
      Then lifecycle завершается с кодом 0
      And attempt завершён terminal event completed

    Scenario: External source через symbolic link копируется до возврата tool call
      Given подготовлен single-step workflow с output result
      When Agent передаёт external source через absolute symbolic link и возвращает управление
      Then lifecycle завершается с кодом 0
      And durable artifact result содержит bytes external
      And symbolic link остался caller-owned после удаления source

    @process
    Scenario Outline: Невалидный artifact-запрос возвращает 3 и не завершает attempt
      Given подготовлен single-step workflow с output result
      And process Agent отправляет невалидный completion "<case>"
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 1
      And дочерний attempt complete завершился с кодом 3
      And initial attempt остаётся незавершённым

      Examples:
        | case |
        | с отсутствующим output |
        | с дополнительным output |
        | с повторяющимся output |
        | с относительным path |
        | с отсутствующим path |
        | с path на directory |

  @cli:session-activate-и-attempt-complete @cli:вывод-команд @format:artifact @format:agent-attempt-record @workflow:циклы-terminal-и-blocked-run
  Rule: Completion становится durable только после возврата Agent
    После возврата Agent последний принятый completion-кандидат финализируется независимо от exit code; без кандидата terminal event не добавляется, и attempt доступен только последующему явному resume.

    Scenario: Single-step run публикует artifact и завершается
      Given подготовлен single-step workflow с output result
      When Agent передаёт completion с bytes final и возвращает управление
      Then lifecycle завершается с кодом 0
      And durable artifact result содержит bytes final
      And attempt завершён terminal event completed
      And lifecycle сообщает о завершении run

    Scenario: Следующий валидный completion целиком заменяет предыдущий кандидат
      Given подготовлен single-step workflow с output result
      When Agent передаёт completion сначала с bytes draft, затем final и возвращает управление
      Then lifecycle завершается с кодом 0
      And durable artifact result содержит bytes final

    Scenario: Невалидный completion не изменяет предыдущий кандидат
      Given подготовлен single-step workflow с output result
      When Agent передаёт валидный completion good, затем невалидный extra и возвращает управление
      Then lifecycle завершается с кодом 0
      And durable artifact result содержит bytes good

    Scenario: Completion финализируется до обработки ненулевого exit агента
      Given подготовлен single-step workflow с output result
      When Agent передаёт completion final и завершается с кодом 7
      Then lifecycle завершается с кодом 1
      And durable artifact result содержит bytes final
      And attempt завершён terminal event completed

    Scenario: Resume завершённого run не запускает Agent
      Given подготовлен завершённый single-step run
      When run продолжается через lifecycle API
      Then lifecycle завершается с кодом 0
      And Agent не запускается повторно
      And lifecycle сообщает already completed

  @cli:session-activate-и-attempt-complete @format:agent-attempt-record @format:artifact
  Rule: Completion-кандидат не закрывает активную обработку attempt
    Пока Agent process не вернул управление, каждый валидный attempt complete целиком заменяет предыдущий кандидат, а session activations продолжают приниматься; parent финализирует последний кандидат только после обработки более ранних control calls.

    Scenario: После completion-кандидата принимаются activation и новый кандидат
      Given подготовлен single-step workflow с output result
      When Agent передаёт completion draft, активирует session late-session, передаёт completion final и возвращается
      Then lifecycle завершается с кодом 0
      And durable artifact result содержит bytes final
      And durable attempt содержит late-session перед completed

  @process @cli:resume @cli:session-activate-и-attempt-complete @format:artifact @format:agent-attempt-record
  Rule: Crash до commit отбрасывает volatile completion-кандидат
    Принятый completion не восстанавливается, пока Agent не вернул управление и completed не опубликован; следующий resume продолжает тот же attempt и может передать новый полный кандидат, а crash-leftovers без completed не входят в durable-модель.

    Scenario: Resume заменяет принятый до SIGKILL кандидат новым completion
      Given подготовлен single-step workflow с output result
      And process Agent передаёт completion draft и блокируется до возврата
      When supervisor завершается SIGKILL до возврата Agent, а resume передаёт final
      Then lifecycle завершается с кодом 0
      And durable artifact result содержит bytes final
      And source draft остался caller-owned вне durable run
      And attempt завершён terminal event completed
