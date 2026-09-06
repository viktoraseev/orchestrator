Feature: Резервирование и блокировка run
  Каждый start создаёт собственный durable run, а выполнять существующий run может только один supervisor.

  @cli:start @cli:коды-завершения @format:корень-состояния-и-layout
  Rule: Start атомарно резервирует новый RunId
    RunId является timestamp-кандидатом в миллисекундах; существующий каталог никогда не переиспользуется и не присоединяется к новому start, а конкурирующие start публикуют разные runs.

    @process
    Scenario: Одновременные start не присоединяются к одному run
      Given подготовлен single-step workflow без outputs
      And process Agent завершает attempt без outputs
      When одновременно запускаются два orchestrator start delivery
      Then оба lifecycle завершаются с кодом 0
      And start публикуют два разных RunId и два независимых durable run

    Scenario: Ошибка резервирования не запускает Agent
      Given подготовлен single-step workflow без outputs
      And путь run занят regular file
      When workflow запускается через lifecycle API с возвратом без completion
      Then lifecycle завершается с кодом 1
      And Agent не запускался и regular file run не изменился

  @process @cli:resume @cli:коды-завершения @format:корень-состояния-и-layout @workflow:планирование
  Rule: Один supervisor удерживает Run lock весь lifecycle
    Start или resume неблокирующе получает exclusive kernel lock до запуска executor и удерживает его между всеми Steps; competing supervisor получает код 5 без durable-изменений, а после exit или crash lock освобождается ядром.

    Scenario: Второй supervisor не получает занятый Run lock
      Given подготовлен single-step workflow без outputs
      And process Agent ожидает явного разрешения
      When во время первого start запускается competing resume
      Then lifecycle завершается с кодом 5
      And competing resume не изменяет initial attempt

    Scenario: Run lock не освобождается между последовательными Steps
      Given подготовлен последовательный workflow без outputs
      And process Agent завершает initial Step и ожидает на target
      When во время первого start запускается competing resume
      Then lifecycle завершается с кодом 5
      And competing resume не изменяет durable run
      When run продолжается через lifecycle API
      Then lifecycle завершается с кодом 1
      And resume запускает незавершённый target attempt 1
