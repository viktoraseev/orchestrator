Feature: Восстановление durable run
  Resume проверяет полную durable-модель до запуска Agent и сохраняет crash leftovers вне модели.

  @spec:создание-и-восстановление-agent-attempt @format:agent-attempt-record @format:artifact @cli:resume
  Rule: Противоречивая durable-модель отклоняется до побочных эффектов
    После получения Run lock и до запуска executor, вычисления frontier или durable publication resume полностью проверяет materialized workflow, все attempt records и artifacts завершённых attempts на соответствие format и workflow graph; любая ошибка завершает команду без запуска Agent и изменения durable-модели.
    После успешной проверки состояние attempt вычисляется из validated materialized workflow, полного attempt record и artifacts; отдельный durable status не сохраняется.

    Scenario Outline: Ошибка любого связанного durable-факта возвращает код 3
      Given подготовлен завершённый линейный durable run
      And durable-модель повреждена как "<повреждение>"
      When повреждённый run продолжается через lifecycle API
      Then lifecycle завершается с кодом 3
      And Agent не запускался и повреждённый durable run не изменился

      Examples:
        | повреждение                         |
        | отсутствующий spec                 |
        | невалидный spec YAML               |
        | невалидный attempt record          |
        | отсутствующий completed artifact   |
        | дополнительный completed artifact  |
        | противоречивый input               |
        | повторный глобальный attempt number |
        | невалидный content artifact        |

    Scenario: Completed attempt восстанавливается без отдельного status
      Given подготовлен завершённый линейный durable run
      When run продолжается через lifecycle API
      Then lifecycle завершается с кодом 0
      And Agent не запускается повторно
      And lifecycle сообщает already completed

  @spec:создание-и-восстановление-agent-attempt @format:корень-состояния-и-layout @format:artifact @cli:resume
  Rule: Файловые остатки незавершённого attempt не входят в durable-модель
    Временные файлы атомарной записи, artifacts без соответствующего attempt и файловые остатки attempt без completion не участвуют в validation, recovery или workflow graph и не удаляются автоматически.

    Scenario: Resume игнорирует и не удаляет temp и orphan artifacts
      Given подготовлен незавершённый durable run с crash leftovers
      When run с crash leftovers продолжается через lifecycle API
      Then lifecycle завершается с кодом 1
      And resume продолжает исходный unfinished attempt
      And crash leftovers остались побайтово неизменными

    Scenario: Новый attempt не переиспользует номер orphan artifact
      Given подготовлен ready durable run с orphan artifact 99
      When ready run с orphan artifact продолжается через lifecycle API
      Then lifecycle завершается с кодом 1
      And target получает следующий свободный глобальный номер 100
