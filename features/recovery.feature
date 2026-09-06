Feature: Восстановление durable run
  Resume после получения Run lock проверяет все опубликованные attempts, выводит их состояние без отдельной state-записи или обязательного output-маркера и только затем продолжает unfinished attempts либо создаёт ready activations; crash leftovers остаются вне модели.

  @format:artifact @cli:resume
  Rule: Противоречивая durable-модель отклоняется до побочных эффектов
    После получения Run lock и до запуска executor, вычисления frontier или durable publication resume полностью проверяет materialized workflow, все attempt records и artifacts завершённых attempts на соответствие format и workflow graph; любая ошибка завершает команду без запуска Agent и изменения durable-модели.
    Attempt record является regular YAML file с именем `<n>.<step-id>.attempt.yaml` и закрытым root mapping из обязательных sequences `input` и `events`; имя связывает record с глобальным номером и существующим Step, duplicate keys, неизвестные поля и нарушения типов запрещены.
    Каждый event является закрытым mapping: session activation содержит только `type: session-activated` и строковый `session-id`, completion — только `type: completed`; соседние session activations не повторяют ID, completed встречается не более одного раза и только последним.
    После успешной проверки состояния running, paused и completed вычисляются из validated workflow, record и artifacts и отдельными полями не сохраняются.

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
        | attempt record без input           |
        | attempt record с неизвестным полем |
        | attempt record с duplicate root key |
        | input не sequence                  |
        | events не sequence                 |
        | неизвестный event type             |
        | session event без session-id       |
        | completion event с лишним полем    |
        | соседний повтор session activation |
        | completed не является последним    |
        | completed повторяется              |
        | невалидное имя attempt record      |
        | нечисловой номер attempt           |
        | attempt с неизвестным Step в имени |
        | attempt record не regular file     |
        | initial attempt с непустым input   |
        | отсутствующий completed artifact   |
        | дополнительный completed artifact  |
        | неполная input group               |
        | отсутствующий input source         |
        | незавершённый input source         |
        | противоречивый input               |
        | повторный глобальный attempt number |
        | невалидный content artifact        |

    Scenario: Completed attempt восстанавливается без отдельного status
      Given подготовлен завершённый линейный durable run
      When run продолжается через lifecycle API
      Then lifecycle завершается с кодом 0
      And Agent не запускается повторно
      And lifecycle сообщает already completed

    Scenario: Старая source version в повторной activation отклоняется
      Given подготовлен циклический durable run со старой input group
      When повреждённый run продолжается через lifecycle API
      Then lifecycle завершается с кодом 3
      And Agent не запускался и повреждённый durable run не изменился

  @format:artifact @cli:resume
  Rule: Номера attempts глобальны и не переиспользуются
    Attempt существует только после атомарной публикации record; первый attempt получает номер 0, а каждый следующий — номер больше любого опубликованного или зарезервированного crash-остатком номера во всём run, поэтому пропуски не заполняются.

    Scenario: Resume создаёт первый attempt 0 после crash до его публикации
      Given подготовлен циклический durable run без attempt records
      When run продолжается через lifecycle API
      Then lifecycle завершается с кодом 1
      And resume публикует и запускает первый attempt 0

    Scenario: Пропуски между опубликованными attempts не заполняются
      Given подготовлен ready durable run с attempts 0, 2 и 4
      When готовый target продолжается через lifecycle API
      Then lifecycle завершается с кодом 1
      And новый join attempt получает номер 5 без заполнения пропусков 1 и 3

    Scenario: Новый attempt не переиспользует номер orphan artifact
      Given подготовлен ready durable run с orphan artifact 99
      When готовый target продолжается через lifecycle API
      Then lifecycle завершается с кодом 1
      And target получает следующий свободный глобальный номер 100

  @format:корень-состояния-и-layout @format:artifact @cli:resume
  Rule: Файловые остатки незавершённого attempt не входят в durable-модель
    Временные файлы атомарной записи, artifacts без соответствующего attempt и файловые остатки attempt без completion не участвуют в validation, recovery или workflow graph и не удаляются автоматически.

    Scenario: Resume игнорирует и не удаляет temp и orphan artifacts
      Given подготовлен незавершённый durable run с crash leftovers
      When run с crash leftovers продолжается через lifecycle API
      Then lifecycle завершается с кодом 1
      And resume продолжает исходный unfinished attempt
      And crash leftovers остались побайтово неизменными
