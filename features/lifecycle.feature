Feature: Durable lifecycle run
  Lifecycle создаёт и продолжает run только через подтверждённую durable-модель.

  @cli:start @format:materialized-workflow @format:agent-attempt-record @spec:сущности @workflow:initial-activation-dependencies-и-frontier
  Rule: Start обещает только durable run
    Возврат Agent process без принятого completion оставляет attempt незавершённым и завершает команду runtime failure; автоматического повторного запуска или native resume в этой lifecycle-команде нет.

    Scenario: Agent возвращает управление без completion
      Given подготовлен single-step workflow без outputs
      When workflow запускается через lifecycle API с возвратом без completion
      Then lifecycle завершается с кодом 1
      And до вызова Agent опубликованы spec и initial attempt 0
      And RunId является десятичным Unix timestamp создания в миллисекундах
      And initial attempt остаётся незавершённым
      And Agent запускался ровно один раз

    @process
    Scenario: CLI печатает durable promise до преждевременного возврата process Agent
      Given подготовлен single-step workflow без outputs
      And process Agent возвращает управление без completion
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 1
      And stdout содержит workflow, RunId и финальную строку exited в стабильном порядке
      And initial attempt остаётся незавершённым

    @process
    Scenario: Второй supervisor не получает занятый Run lock
      Given подготовлен single-step workflow без outputs
      And process Agent ожидает явного разрешения
      When во время первого start запускается competing resume
      Then lifecycle завершается с кодом 5
      And competing resume не изменяет initial attempt

    Scenario: Несовместимый Agent type отклоняется до создания run
      Given подготовлен single-step workflow без outputs
      When workflow запускается через lifecycle API с Agent type без native resume
      Then lifecycle завершается с кодом 3
      And lifecycle run не создан

  @cli:resume
  Rule: Resume требует явный RunId
    Resume без RunId не выбирает последний run и отклоняется CLI parser; неизвестный явно указанный RunId не создаёт run.

    @process
    Scenario: Resume без RunId не продолжает существующий run
      Given подготовлен незавершённый run без session activations
      When запускается orchestrator resume без RunId
      Then lifecycle завершается с кодом 2
      And Agent не запускался и durable run не изменился

    Scenario: Неизвестный run не создаётся при resume
      Given подготовлен корень без runs
      When неизвестный run 404 продолжается через lifecycle API
      Then lifecycle завершается с кодом 4
      And каталог неизвестного run не создан

  @cli:resume @format:agent-attempt-record @spec:control-endpoint-и-события
  Rule: Session activation сохраняется в durable-порядке
    Явный resume продолжает тот же attempt с последней Agent session activation в durable-порядке независимо от create, resume или fork; без activation Agent type создаёт новую внутреннюю session для того же attempt и номера.

    Scenario: Resume без activation продолжает тот же attempt с новой session
      Given подготовлен незавершённый run без session activations
      When run продолжается через lifecycle API
      Then lifecycle завершается с кодом 1
      And resume запускает тот же attempt 0 без session activation

    Scenario: Resume использует последнюю отличающуюся непрозрачную native session
      Given подготовлен single-step workflow без outputs
      When Agent активирует opaque sessions vendor/a:1, vendor/b:2, vendor/a:1, vendor/a:1 и возвращается без completion
      And run продолжается через lifecycle API
      Then resume запускает тот же attempt 0 с session vendor/a:1
      And durable activations равны vendor/a:1, vendor/b:2, vendor/a:1
      And durable activations не содержат вид create, resume или fork

    @process
    Scenario: Process Agent сохраняет session через дочернюю CLI-команду
      Given подготовлен single-step workflow без outputs
      And process Agent активирует session process-session
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 1
      And durable activations равны process-session

    @process
    Scenario: Process resume получает последнюю durable session ID
      Given подготовлен single-step workflow без outputs
      When process Agent активирует session и следующий resume её продолжает
      Then lifecycle завершается с кодом 0
      And attempt завершён terminal event completed

  @cli:resume @workflow:validation
  Rule: Resume не заменяет несовместимый Agent type новой session
    Потеря поддержки native resume материализованным Agent type завершает resume fail-fast до запуска Agent и без интерактивных вопросов; fallback на новую session запрещён.

    Scenario: Существующий run отклоняется до запуска несовместимого Agent
      Given подготовлен незавершённый run без session activations
      When run продолжается с Agent type без native resume
      Then lifecycle завершается с кодом 3
      And Agent не запускался и durable run не изменился

  @cli:session-activate-и-attempt-complete @cli:вывод-команд @format:artifact @format:agent-attempt-record @spec:публикация-artifacts-и-completion @workflow:циклы-terminal-и-blocked-run
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

    Scenario: Resume завершённого run не запускает Agent
      Given подготовлен завершённый single-step run
      When run продолжается через lifecycle API
      Then lifecycle завершается с кодом 0
      And Agent не запускается повторно
      And lifecycle сообщает already completed

    @process
    Scenario: Process Agent завершает attempt дочерней CLI-командой
      Given подготовлен single-step workflow с output result
      And process Agent публикует artifact final через attempt complete
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 0
      And дочерний orchestrator использовал унаследованные ORC_HOME, ORC_CONTROL_ENDPOINT, ORC_RUN_ID и ORC_ATTEMPT
      And durable artifact result содержит bytes final
      And lifecycle сообщает о завершении run
