Feature: Конфигурация orchestrator
  Конфигурация читается только из выбранного корня состояния и имеет стабильное CLI-представление.

  @cli:config-get-config-set-и-config-list @process
  Rule: Отсутствующая конфигурация имеет документированные эффективные значения

    Scenario Outline: Чтение значения без config.yaml
      Given изолированный корень состояния без config.yaml
      When я выполняю config get <key>
      Then команда завершается с кодом 0
      And stdout равен строке "<value>"

      Examples:
        | key                 | value |
        | default-workflow    | null  |
        | default-agent       | null  |
        | max-parallel-agents | 5     |

    Scenario: Чтение полного списка без config.yaml
      Given изолированный корень состояния без config.yaml
      When я выполняю config list
      Then команда завершается с кодом 0
      And stdout равен строкам default-workflow null, default-agent null и max-parallel-agents 5

  @process
  Rule: ORC_HOME целиком задаёт корень состояния
    При отсутствующем или пустом ORC_HOME корнем является только `$HOME/.orc`; непустой ORC_HOME обязан быть absolute path, полностью заменяет default root и не добавляет поиск в current directory или fallback-пути.

    Scenario: Без ORC_HOME используется HOME/.orc без fallback в current directory
      Given HOME/.orc содержит лимит 7, current directory содержит лимит 11, а ORC_HOME отсутствует
      When я выполняю config get max-parallel-agents
      Then команда завершается с кодом 0
      And stdout равен строке "7"

    Scenario: Абсолютный ORC_HOME заменяет корень из HOME
      Given HOME содержит лимит 7, а абсолютный ORC_HOME содержит лимит 11
      When я выполняю config get max-parallel-agents
      Then команда завершается с кодом 0
      And stdout равен строке "11"

    Scenario: Пустой ORC_HOME не заменяет корень из HOME
      Given HOME содержит лимит 7, а ORC_HOME пуст
      When я выполняю config get max-parallel-agents
      Then команда завершается с кодом 0
      And stdout равен строке "7"

    Scenario: Относительный ORC_HOME отклоняется
      Given относительный ORC_HOME
      When я выполняю config list
      Then команда завершается с кодом 3
      And stderr начинается с "error: config:"

  @cli:config-get-config-set-и-config-list @process
  Rule: Ошибки config-команд не маскируются значениями по умолчанию
    Необязательный `<root>/config.yaml` при наличии является закрытым YAML mapping: duplicate keys, неизвестные поля, нарушения field types и форматов symbolic IDs делают весь config невалидным до применения defaults или вывода.

    Scenario: Config со всеми четырьмя поддерживаемыми полями валиден
      Given config.yaml со всеми поддерживаемыми полями
      When я выполняю config list
      Then команда завершается с кодом 0
      And stdout равен строкам default-workflow delivery, default-agent codex-main и max-parallel-agents 7

    Scenario: Неизвестный ключ отклоняется CLI parser
      Given изолированный корень состояния без config.yaml
      When я выполняю config get unknown
      Then команда завершается с кодом 2

    Scenario: Неизвестное поле config отклоняется
      Given config.yaml с неизвестным полем
      When я выполняю config list
      Then команда завершается с кодом 3
      And stderr начинается с "error: config:"

    Scenario Outline: Config schema и Agent registry проверяются целиком
      Given config.yaml с нарушением schema "<case>"
      When я выполняю config list
      Then команда завершается с кодом 3
      And stderr начинается с "error: config:"

      Examples:
        | case |
        | root is sequence |
        | duplicate root field |
        | default-workflow is not string |
        | default-agent has repeated hyphen |
        | max-parallel-agents is zero |
        | agents is not mapping |
        | AgentId contains dot |
        | Agent misses type |
        | Agent model is not string |
        | Agent has command field |
        | Agent has environment field |
        | Agent type is unknown |
        | Agent model is invalid |
        | Agent reasoning is invalid |

  @cli:config-get-config-set-и-config-list @process
  Rule: max-parallel-agents изменяется атомарно

    Scenario: Положительный лимит сохраняется
      Given изолированный корень состояния без config.yaml
      When я выполняю config set max-parallel-agents 7
      Then команда завершается с кодом 0
      And stdout равен строке "max-parallel-agents: 7"
      And config get max-parallel-agents возвращает "7"

    Scenario Outline: Невалидный лимит не изменяет config
      Given config.yaml с лимитом 7
      When я выполняю config set max-parallel-agents <value>
      Then команда завершается с кодом 3
      And config.yaml остался побайтово неизменным

      Examples:
        | value |
        | 0     |
        | -1    |
        | nope  |

    Scenario: Успешная запись сохраняет остальные поля
      Given config.yaml со всеми поддерживаемыми полями
      When я выполняю config set max-parallel-agents 11
      Then команда завершается с кодом 0
      And остальные поля config.yaml сохранены

    Scenario: Конкурентные записи оставляют целый config
      Given изолированный корень состояния без config.yaml
      When конкурентно устанавливаются лимиты 13 и 17
      Then обе команды завершаются с кодом 0
      And итоговый лимит равен 13 или 17

    Scenario: Неизвестный ключ set не изменяет config
      Given config.yaml с лимитом 7
      When я выполняю config set unknown 9
      Then команда завершается с кодом 2
      And config.yaml остался побайтово неизменным

    Scenario: Отсутствующее значение set не изменяет config
      Given config.yaml с лимитом 7
      When я выполняю config set max-parallel-agents без значения
      Then команда завершается с кодом 2
      And config.yaml остался побайтово неизменным

  @cli:config-get-config-set-и-config-list @process
  Rule: default-workflow ссылается на существующий workflow template

    Scenario: Существующий workflow выбирается без materialization
      Given workflow delivery с невалидным содержимым
      When я выполняю config set default-workflow delivery
      Then команда завершается с кодом 0
      And stdout равен строке "default-workflow: delivery"
      And config get default-workflow возвращает "delivery"

    Scenario: Отсутствующий workflow не изменяет config
      Given config.yaml с лимитом 7
      When я выполняю config set default-workflow missing
      Then команда завершается с кодом 4
      And config.yaml остался побайтово неизменным

    Scenario: Невалидный WorkflowId не изменяет config
      Given config.yaml с лимитом 7
      When я выполняю config set default-workflow Bad-ID
      Then команда завершается с кодом 2
      And config.yaml остался побайтово неизменным

    Scenario: Выбор workflow сохраняет остальные поля
      Given config.yaml со всеми поддерживаемыми полями
      And существует workflow research
      When я выполняю config set default-workflow research
      Then команда завершается с кодом 0
      And config.yaml сохраняет Agent и лимит при выборе workflow research

  @cli:config-get-config-set-и-config-list @process
  Rule: default-agent ссылается на существующего именованного Agent

    Scenario: Существующий Agent выбирается по умолчанию
      Given config.yaml с двумя именованными Agents
      When я выполняю config set default-agent claude-main
      Then команда завершается с кодом 0
      And stdout равен строке "default-agent: claude-main"
      And config get default-agent возвращает "claude-main"
      And config.yaml сохраняет workflow, лимит и обоих Agents

    Scenario: Отсутствующий Agent не изменяет config
      Given config.yaml с двумя именованными Agents
      When я выполняю config set default-agent missing
      Then команда завершается с кодом 4
      And config.yaml остался побайтово неизменным

    Scenario: Невалидный AgentId не изменяет config
      Given config.yaml с двумя именованными Agents
      When я выполняю config set default-agent Bad-ID
      Then команда завершается с кодом 2
      And config.yaml остался побайтово неизменным

    Scenario Outline: Невалидный Agent не допускает публикацию config
      Given config.yaml с невалидным Agent из-за <field>
      When я выполняю config set default-agent codex-main
      Then команда завершается с кодом 3
      And config.yaml остался побайтово неизменным

      Examples:
        | field     |
        | type      |
        | model     |
        | reasoning |
