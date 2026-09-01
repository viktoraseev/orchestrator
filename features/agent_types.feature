Feature: Встроенные Agent types

  @process @spec:сущности @spec:главный-workflow @cli:корень-состояния-и-переменные-окружения @cli:вывод-команд
  Rule: Codex adapter строит и интерпретирует собственный protocol

    Scenario: Новая non-human Codex session получает точные args и environment
      Given подготовлен single-step workflow для Agent type codex
      And process Agent записывает args и environment и завершает attempt
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 0
      And process Agent получил точные args для codex
      And process Agent получил полный control environment
      And сырой Agent protocol отсутствует в выводе

  @process @spec:сущности @spec:control-endpoint-и-события @spec:главный-workflow @cli:вывод-команд
  Rule: Claude adapter строит и интерпретирует собственный protocol

    Scenario: Новая non-human Claude session получает точные args и environment
      Given подготовлен single-step workflow для Agent type claude
      And process Agent записывает args и environment и завершает attempt
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 0
      And process Agent получил точные args для claude
      And process Agent получил полный control environment
      And сырой Agent protocol отсутствует в выводе

  @process @spec:сущности @spec:native-resume @spec:главный-workflow @cli:вывод-команд
  Rule: Обязательный session event интерпретируется fail-fast

    Scenario Outline: Невалидный protocol не создаёт выдуманную session
      Given подготовлен single-step workflow для Agent type <type>
      And process Agent для <type> возвращает <case> protocol
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 1
      And protocol failure не создал выдуманную session

      Examples:
        | type   | case          |
        | codex  | malformed     |
        | codex  | missing       |
        | codex  | contradictory |
        | claude | malformed     |
        | claude | missing       |
        | claude | contradictory |

  @process @spec:сущности @cli:корень-состояния-и-переменные-окружения
  Rule: Executable override не выполняет PATH lookup

    Scenario: Отсутствующий absolute executable отклоняется до запуска
      Given подготовлен single-step workflow для Agent type codex
      And ORC_AGENT_COMMAND указывает на отсутствующий absolute path
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 3
      And initial attempt не создан

  @process @spec:сущности @spec:главный-workflow @cli:вывод-команд
  Rule: Agent session view остаётся volatile

    Scenario Outline: TTY board нормализует сообщения встроенного Agent type
      Given подготовлен single-step workflow для Agent type <type>
      And process Agent публикует два сообщения для <type> и завершает attempt
      When workflow запускается через pseudo-terminal
      Then lifecycle завершается с кодом 0
      And TTY board показывает 2 и последнее сообщение одной строкой
      And session view отсутствует в durable run

      Examples:
        | type   |
        | codex  |
        | claude |

    Scenario: Без TTY board не создаётся
      Given подготовлен single-step workflow для Agent type codex
      And process Agent публикует два сообщения для codex и завершает attempt
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 0
      And Agent board отсутствует в выводе
