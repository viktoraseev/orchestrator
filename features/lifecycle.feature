Feature: Durable lifecycle run
  Run — один сохраняемый запуск materialized workflow; attempts и artifacts образуют его durable-модель, а текущая позиция и состояние вычисляются из неё. Lifecycle создаёт и продолжает run только через подтверждённую durable-модель.

  @cli:start
  Rule: Start обещает только durable run
    После общего preflight start резервирует и блокирует run, durable-публикует проверенный materialized workflow до initial attempt и запускает Agent только после обеих публикаций; возврат Agent process без принятого completion оставляет attempt незавершённым и завершает команду runtime failure без автоматического повторного запуска или native resume в этой lifecycle-команде.
    Durable run находится только в `<root>/run/<run-id>`: materialized workflow публикуется как `spec.yaml`, attempts как `<n>.<step-id>.attempt.yaml`, а artifacts как `<n>.<step-id>.<input-id>.artifact`.
    До резервирования RunId source workflow, prompts и Agents materialize’ятся только в памяти; `spec.yaml` содержит ровно `workflow-id`, effective `max-parallel-agents`, mapping всех run parameters и Steps в source order, а каждый Agent Step — ровно `id`, materialized `agent`, точный `prompt` либо null, `human`, `process: null`, `depends-on` и `outputs`.
    Durable snapshot не содержит AgentId, PromptId или ссылок на изменяемые config, workflow и prompt files, публикуется атомарно под фиксированным именем после получения Run lock и остаётся неизменным при resume.
    Initial attempt имеет глобальный номер 0 и имя `0.<first-step-id>.attempt.yaml`; его закрытый YAML mapping содержит только пустые sequences `input` и `events`, не materialize'ит данные workflow и не хранит производные статусы.

    Scenario: Agent возвращает управление без completion
      Given подготовлен single-step workflow без outputs
      When workflow запускается через lifecycle API с возвратом без completion
      Then lifecycle завершается с кодом 1
      And до вызова Agent опубликованы spec и initial attempt 0
      And durable spec содержит закрытую Agent Step schema без source references
      And initial attempt record содержит только пустые input и events
      And RunId является десятичным Unix timestamp создания в миллисекундах
      And initial attempt остаётся незавершённым
      And Agent запускался ровно один раз

    Scenario: Resume не перечитывает изменённые source definitions
      Given подготовлен single-step workflow с prompt original
      When start materialize'ит workflow, а source definitions изменяются перед resume
      Then lifecycle завершается с кодом 1
      And resume использует сохранённые Agent, prompt и topology, а spec неизменен

    Scenario: NUL в run parameter отклоняется до публикации
      Given подготовлен single-step workflow с обязательным parameter mode
      When lifecycle API получает parameter mode с NUL
      Then lifecycle завершается с кодом 3
      And lifecycle run не создан

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

  @cli:resume
  Rule: Session activation сохраняется в durable-порядке
    Явный resume продолжает тот же attempt с последней Agent session activation в durable-порядке независимо от create, resume или fork; без activation Agent type создаёт новую внутреннюю session для того же attempt и номера.
    Session activation записывается mapping с ровно `type: session-activated` и строковым `session-id`; последовательность `events` хранит durable-порядок, а повтор последнего ID является успешным no-op и не добавляет событие.

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
