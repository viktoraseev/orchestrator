Feature: Durable lifecycle run
  Run — один сохраняемый запуск materialized workflow; attempts и artifacts образуют его durable-модель, а текущая позиция и состояние вычисляются из неё. Lifecycle создаёт и продолжает run только через подтверждённую durable-модель.

  @cli:start @format:materialized-workflow @format:agent-attempt-record
  Rule: Start обещает только durable run
    После общего preflight start резервирует и блокирует run, durable-публикует проверенный materialized workflow до initial attempt и запускает Agent только после обеих публикаций; возврат Agent process без принятого completion оставляет attempt незавершённым и завершает команду runtime failure без автоматического повторного запуска или native resume в этой lifecycle-команде.
    Source workflow, prompts и Agents проверяются и materialize’ятся в памяти до создания run; опубликованный snapshot сохраняет порядок Steps для scheduling и не меняется вслед за source files.

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

  @cli:resume @format:agent-attempt-record
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
      And session activation не создала второй run

    @process
    Scenario: Process resume получает последнюю durable session ID
      Given подготовлен single-step workflow без outputs
      When process Agent активирует session и следующий resume её продолжает
      Then lifecycle завершается с кодом 0
      And attempt завершён terminal event completed

  @cli:resume
  Rule: Resume не заменяет несовместимый Agent type новой session
    Потеря поддержки native resume материализованным Agent type завершает resume fail-fast до запуска Agent и без интерактивных вопросов; fallback на новую session запрещён.

    Scenario: Существующий run отклоняется до запуска несовместимого Agent
      Given подготовлен незавершённый run без session activations
      When run продолжается с Agent type без native resume
      Then lifecycle завершается с кодом 3
      And Agent не запускался и durable run не изменился
