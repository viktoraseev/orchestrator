Feature: Durable lifecycle run
  Lifecycle создаёт и продолжает run только через подтверждённую durable-модель.

  @cli:start @format:materialized-workflow @format:agent-attempt-record @spec:сущности @workflow:initial-activation-dependencies-и-frontier
  Rule: Start обещает только durable run

    Scenario: Agent возвращает управление без completion
      Given подготовлен single-step workflow без outputs
      When workflow запускается через lifecycle API с возвратом без completion
      Then lifecycle завершается с кодом 1
      And до вызова Agent опубликованы spec и initial attempt 0
      And RunId является десятичным Unix timestamp создания в миллисекундах
      And initial attempt остаётся незавершённым

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

  @cli:resume @format:agent-attempt-record @spec:native-resume @spec:control-endpoint-и-события
  Rule: Session activation сохраняется в durable-порядке

    Scenario: Resume использует последнюю отличающуюся непрозрачную native session
      Given подготовлен single-step workflow без outputs
      When Agent активирует opaque sessions vendor/a:1, vendor/b:2, vendor/a:1, vendor/a:1 и возвращается без completion
      And run продолжается через lifecycle API
      Then resume запускает тот же attempt 0 с session vendor/a:1
      And durable activations равны vendor/a:1, vendor/b:2, vendor/a:1

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

    Scenario: Неизвестный run не создаётся при resume
      Given подготовлен корень без runs
      When неизвестный run 404 продолжается через lifecycle API
      Then lifecycle завершается с кодом 4
      And каталог неизвестного run не создан

  @cli:session-activate-и-attempt-complete @cli:вывод-команд @format:artifact @format:agent-attempt-record @spec:публикация-artifacts-и-completion @workflow:циклы-terminal-и-blocked-run
  Rule: Completion становится durable только после возврата Agent

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
      And durable artifact result содержит bytes final
      And lifecycle сообщает о завершении run
