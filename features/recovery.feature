Feature: Восстановление durable run
  Resume проверяет полную durable-модель до запуска Agent и сохраняет crash leftovers вне модели.

  @spec:fail-fast-восстановление @spec:создание-и-восстановление-agent-attempt @format:agent-attempt-record @format:artifact @cli:resume
  Rule: Противоречивая durable-модель отклоняется до побочных эффектов

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

  @spec:создание-и-восстановление-agent-attempt @spec:fail-fast-восстановление @format:корень-состояния-и-layout @format:artifact @cli:resume
  Rule: Файловые остатки незавершённого attempt не входят в durable-модель

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
