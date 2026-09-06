Feature: Read-only catalogs source definitions
  Catalog commands читают только выбранный state root, не materialize'ят workflow, не создают run и не изменяют файлы; весь catalog проверяется до renderer, поэтому ошибка не даёт partial stdout.

  @format:корень-состояния-и-layout @format:идентификаторы-и-номера @cli:source-catalogs
  Rule: Workflow catalog перечисляет source templates без materialization

    Scenario: Пустой workflow catalog успешен через публичный API
      Given подготовлен пустой source catalog root
      When workflow catalog строится через публичный API
      Then catalog завершается с кодом 0
      And catalog output пуст
      When запускается orchestrator workflow list в JSON
      Then catalog завершается с кодом 0
      And JSON catalog равен пустому array

    @process
    Scenario: Workflow templates сортируются и имеют абсолютные paths в JSON
      Given подготовлены workflow templates beta и alpha с посторонними entries
      When запускается orchestrator workflow list в JSON
      Then catalog завершается с кодом 0
      And JSON workflow catalog равен alpha и beta с absolute paths по порядку
      And catalog не изменил source state

    @process
    Scenario: Workflow text catalog содержит только ID по порядку
      Given подготовлены workflow templates beta и alpha с посторонними entries
      When запускается orchestrator workflow list
      Then catalog завершается с кодом 0
      And workflow catalog text равен строкам alpha и beta

    @process
    Scenario: Невалидный WorkflowId contract file не даёт partial stdout
      Given подготовлен workflow contract file с невалидным ID
      When запускается orchestrator workflow list
      Then catalog завершается с кодом 3
      And catalog output пуст

    @process
    Scenario: Workflow contract path обязан быть regular file
      Given подготовлен workflow contract path, который является directory
      When запускается orchestrator workflow list
      Then catalog завершается с кодом 3
      And catalog output пуст

  @format:config-yaml @cli:source-catalogs
  Rule: Agent catalog использует полную config validation

    @process
    Scenario: Отсутствующий config даёт пустой Agent catalog
      Given подготовлен пустой source catalog root
      When запускается orchestrator agent list в JSON
      Then catalog завершается с кодом 0
      And JSON catalog равен пустому array

    Scenario: Typed Agent catalog сохраняет параметры named Agents
      Given подготовлен config с Agents beta и alpha
      When Agent catalog строится через публичный API
      Then catalog завершается с кодом 0
      And typed Agent catalog равен alpha codex gpt high и beta claude opus high по порядку

    @process
    Scenario: Agent text renderer детерминирован
      Given подготовлен config с Agents beta и alpha
      When запускается orchestrator agent list
      Then catalog завершается с кодом 0
      And Agent catalog text равен строкам alpha codex gpt high и beta claude opus high
      And catalog не изменил source state

    @process
    Scenario: Agent JSON catalog имеет стабильную schema
      Given подготовлен config с Agents beta и alpha
      When запускается orchestrator agent list в JSON
      Then catalog завершается с кодом 0
      And JSON Agent catalog равен alpha codex gpt high и beta claude opus high по порядку

    @process
    Scenario: Невалидный скрытый config field не даёт partial stdout
      Given подготовлен невалидный config для Agent catalog
      When запускается orchestrator agent list в JSON
      Then catalog завершается с кодом 3
      And catalog output пуст

  @format:идентификаторы-и-номера @cli:source-catalogs
  Rule: Prompt catalog полностью читает UTF-8 templates
    Prompt catalog читает каждый regular `.md` contract file как произвольный UTF-8 Markdown без YAML-декодирования и считает точное число bytes; non-UTF-8 template делает весь catalog невалидным до вывода.

    Scenario: Typed prompt catalog считает UTF-8 bytes
      Given подготовлен prompt template unicode с содержимым Привет
      When prompt catalog строится через публичный API
      Then catalog завершается с кодом 0
      And typed prompt unicode имеет размер 12 bytes

    @process
    Scenario: Prompt JSON catalog сортируется и игнорирует temporary entries
      Given подготовлены prompt templates beta и alpha с temporary entry
      When запускается orchestrator prompt list в JSON
      Then catalog завершается с кодом 0
      And JSON prompt catalog равен alpha 10 bytes и beta 4 bytes с absolute paths по порядку
      And catalog не изменил source state

    @process
    Scenario: Prompt text catalog имеет стабильную форму
      Given подготовлены prompt templates beta и alpha с temporary entry
      When запускается orchestrator prompt list
      Then catalog завершается с кодом 0
      And prompt catalog text равен alpha 10 bytes и beta 4 bytes с absolute paths по порядку

    @process
    Scenario: Отсутствующий prompt catalog успешен в обоих форматах
      Given подготовлен пустой source catalog root
      When запускается orchestrator prompt list
      Then catalog завершается с кодом 0
      And catalog output пуст
      When запускается orchestrator prompt list в JSON
      Then catalog завершается с кодом 0
      And JSON catalog равен пустому array

    @process
    Scenario: Non-UTF-8 prompt не даёт partial stdout
      Given подготовлен non-UTF-8 prompt template
      When запускается orchestrator prompt list
      Then catalog завершается с кодом 3
      And catalog output пуст

    @process
    Scenario: Prompt contract file требует валидный PromptId в basename
      Given подготовлен prompt contract file с невалидным ID
      When запускается orchestrator prompt list
      Then catalog завершается с кодом 3
      And catalog output пуст

    @process
    Scenario: Prompt contract path обязан быть regular file
      Given подготовлен prompt contract path, который является directory
      When запускается orchestrator prompt list
      Then catalog завершается с кодом 3
      And catalog output пуст

  @format:идентификаторы-и-номера @cli:source-catalogs
  Rule: Workflow show читает source structure без materialization
    Workflow show выбирает ровно один source template, проверяет его структурную schema и symbolic IDs, сохраняет исходный порядок Steps и source references, но не читает config или prompts, не применяет defaults и не проверяет существование references либо graph reachability.

    Scenario: Typed source workflow сохраняет неразрешённые references и порядок Steps
      Given подготовлен source workflow demo с несуществующими Agent и prompt references
      When workflow demo читается через публичный API
      Then catalog завершается с кодом 0
      And typed source workflow содержит исходные Steps и references
      And catalog не изменил source state

    @process
    Scenario: Workflow show JSON возвращает один typed object
      Given подготовлен source workflow demo с несуществующими Agent и prompt references
      When запускается orchestrator workflow show demo в JSON
      Then catalog завершается с кодом 0
      And JSON workflow show содержит абсолютный path и Steps в source order

    @process
    Scenario: Workflow show text показывает source fields в порядке Steps
      Given подготовлен source workflow demo с несуществующими Agent и prompt references
      When запускается orchestrator workflow show demo
      Then catalog завершается с кодом 0
      And workflow show text содержит header и обе source Step строки

    @process
    Scenario: Невалидная schema выбранного workflow не даёт stdout
      Given подготовлен source workflow demo с пустым списком Steps
      When запускается orchestrator workflow show demo
      Then catalog завершается с кодом 3
      And catalog output пуст

    @process
    Scenario: Неизвестный workflow возвращает not found
      Given подготовлен пустой source catalog root
      When запускается orchestrator workflow show missing
      Then catalog завершается с кодом 4
      And catalog output пуст

    @process
    Scenario: Невалидный WorkflowId отклоняется до filesystem lookup
      Given подготовлен пустой source catalog root
      When запускается orchestrator workflow show с невалидным ID
      Then catalog завершается с кодом 2
      And catalog output пуст

  @format:config-yaml @format:идентификаторы-и-номера @cli:source-catalogs
  Rule: Agent show выбирает Agent только после полной config validation

    Scenario: Typed Agent show возвращает выбранную validated запись
      Given подготовлен config с Agents beta и alpha
      When Agent alpha читается через публичный API
      Then catalog завершается с кодом 0
      And typed Agent show равен alpha codex gpt high

    @process
    Scenario: Agent show JSON возвращает один object
      Given подготовлен config с Agents beta и alpha
      When запускается orchestrator agent show beta в JSON
      Then catalog завершается с кодом 0
      And JSON Agent show равен beta claude opus high
      And catalog не изменил source state

    @process
    Scenario: Agent show text имеет стабильную форму
      Given подготовлен config с Agents beta и alpha
      When запускается orchestrator agent show alpha
      Then catalog завершается с кодом 0
      And Agent show text равен строке agent alpha: type=codex model=gpt reasoning=high

    @process
    Scenario: Невалидный AgentId отклоняется до чтения повреждённого config
      Given подготовлен невалидный config для Agent catalog
      When запускается orchestrator agent show с невалидным ID
      Then catalog завершается с кодом 2
      And catalog output пуст

    @process
    Scenario: Повреждение config не даёт partial stdout Agent show
      Given подготовлен невалидный config для Agent catalog
      When запускается orchestrator agent show alpha
      Then catalog завершается с кодом 3
      And catalog output пуст

    @process
    Scenario: Неизвестный Agent возвращает not found
      Given подготовлен config с Agents beta и alpha
      When запускается orchestrator agent show missing
      Then catalog завершается с кодом 4
      And catalog output пуст

  @format:идентификаторы-и-номера @cli:source-catalogs
  Rule: Prompt show читает только выбранный template и сохраняет точные bytes
    Prompt show возвращает выбранный arbitrary UTF-8 Markdown побайтово, не применяя YAML-декодирование и не добавляя финальный newline.

    Scenario: YAML-похожий Markdown остаётся неразобранным текстом
      Given подготовлен prompt demo с YAML-похожим Markdown
      When prompt demo читается через публичный API
      Then typed prompt show содержит точный YAML-похожий Markdown

    Scenario: Typed prompt show возвращает content и размер выбранного template
      Given подготовлен prompt demo без финального newline и повреждённый соседний template
      When prompt demo читается через публичный API
      Then catalog завершается с кодом 0
      And typed prompt show содержит точное содержимое demo
      And catalog не изменил source state

    @process
    Scenario: Prompt show text не добавляет финальный newline
      Given подготовлен prompt demo без финального newline и повреждённый соседний template
      When запускается orchestrator prompt show demo
      Then catalog завершается с кодом 0
      And prompt show stdout побайтово равен template без newline

    @process
    Scenario: Prompt show JSON сохраняет content и числовой bytes
      Given подготовлен prompt demo с финальным newline
      When запускается orchestrator prompt show demo в JSON
      Then catalog завершается с кодом 0
      And JSON prompt show содержит точный content и bytes

    @process
    Scenario: Выбранный non-UTF-8 prompt не даёт partial stdout
      Given подготовлен non-UTF-8 prompt template
      When запускается orchestrator prompt show binary
      Then catalog завершается с кодом 3
      And catalog output пуст

    @process
    Scenario: Неизвестный prompt возвращает not found
      Given подготовлен пустой source catalog root
      When запускается orchestrator prompt show missing
      Then catalog завершается с кодом 4
      And catalog output пуст

    @process
    Scenario: Невалидный PromptId отклоняется до чтения повреждённого соседа
      Given подготовлен prompt demo без финального newline и повреждённый соседний template
      When запускается orchestrator prompt show с невалидным ID
      Then catalog завершается с кодом 2
      And catalog output пуст
