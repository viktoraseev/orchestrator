Feature: Встроенные Agent types
  Agent type — встроенный adapter запуска, native resume и интерпретации protocol; каждый Agent process получает ORC_STEP_ID, ORC_CONTROL_ENDPOINT, ORC_RUN_ID, ORC_ATTEMPT и полный YAML input mapping в ORC_INPUT поверх type-specific prompt, model, reasoning и native resume arguments.

  @process
  Rule: Codex type проверяет конфигурацию и имеет default executable
    Type `codex` принимает непустой model и reasoning `low`, `medium`, `high`, `xhigh` или `max`; без override запускается executable `codex`.

    Scenario Outline: Codex type принимает только поддерживаемые model и reasoning
      Given config содержит Agent type codex с model <model> и reasoning <reasoning>
      When config проверяется встроенным Agent registry
      Then проверка config завершается с кодом <code>

      Examples:
        | model | reasoning | code |
        | model | low       | 0    |
        | model | medium    | 0    |
        | model | high      | 0    |
        | model | xhigh     | 0    |
        | model | max       | 0    |
        | empty | high      | 3    |
        | model | empty     | 3    |
        | model | ultra     | 3    |

    Scenario: Codex type без override запускает executable codex
      Given подготовлен single-step workflow для Agent type codex
      And в изолированном PATH доступен fake executable codex
      When workflow запускается без ORC_AGENT_COMMAND
      Then lifecycle завершается с кодом 0
      And был запущен default executable codex

  @process
  Rule: Claude type проверяет конфигурацию и имеет default executable
    Type `claude` принимает непустой model и reasoning `low`, `medium`, `high`, `xhigh` или `max`; без override запускается executable `claude`.

    Scenario Outline: Claude type принимает только поддерживаемые model и reasoning
      Given config содержит Agent type claude с model <model> и reasoning <reasoning>
      When config проверяется встроенным Agent registry
      Then проверка config завершается с кодом <code>

      Examples:
        | model | reasoning | code |
        | model | low       | 0    |
        | model | medium    | 0    |
        | model | high      | 0    |
        | model | xhigh     | 0    |
        | model | max       | 0    |
        | empty | high      | 3    |
        | model | empty     | 3    |
        | model | ultra     | 3    |

    Scenario: Claude type без override запускает executable claude
      Given подготовлен single-step workflow для Agent type claude
      And в изолированном PATH доступен fake executable claude
      When workflow запускается без ORC_AGENT_COMMAND
      Then lifecycle завершается с кодом 0
      And был запущен default executable claude

  @process @cli:config-get-config-set-и-config-list @cli:resume
  Rule: Run продолжает materialized Agent независимо от текущего config
    Step или default-agent выбирает именованный Agent только при materialization; его type, model и reasoning входят в durable workflow, поэтому последующее изменение config влияет только на новые runs.

    Scenario Outline: Изменение Agent config не меняет native resume существующего run
      Given подготовлен single-step workflow для Agent type <type>
      And process Agent сначала активирует session, а на resume записывает args и завершает attempt
      When start материализует Agent, config изменяется и run продолжается
      Then lifecycle завершается с кодом 0
      And process Agent получил точные resume args для <type>

      Examples:
        | type   |
        | codex  |
        | claude |

  @process @cli:корень-состояния-и-переменные-окружения @cli:вывод-команд
  Rule: Codex adapter строит и интерпретирует собственный protocol
    Новая non-human session получает args `exec --json --model model --config model_reasoning_effort="high" <prompt>`, а resume вставляет `resume` и native session ID.

    Scenario: Новая non-human Codex session получает точные args и environment
      Given подготовлен single-step workflow для Agent type codex
      And process Agent записывает args и environment и завершает attempt
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 0
      And process Agent получил точные args для codex
      And process Agent получил полный control environment
      And сырой Agent protocol отсутствует в выводе

    Scenario: Native resume Codex получает прежнюю session в точной позиции args
      Given подготовлен single-step workflow для Agent type codex
      And process Agent сначала активирует session, а на resume записывает args и завершает attempt
      When запускаются start и resume через process Agent
      Then lifecycle завершается с кодом 0
      And process Agent получил точные resume args для codex

  @process @cli:вывод-команд
  Rule: Claude adapter строит и интерпретирует собственный protocol
    Новая non-human session получает args `--print --output-format stream-json --verbose --model model --effort high <prompt>`, а resume добавляет `--resume <session-id>` перед prompt.

    Scenario: Новая non-human Claude session получает точные args и environment
      Given подготовлен single-step workflow для Agent type claude
      And process Agent записывает args и environment и завершает attempt
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 0
      And process Agent получил точные args для claude
      And process Agent получил полный control environment
      And сырой Agent protocol отсутствует в выводе

    Scenario: Native resume Claude получает прежнюю session в точной позиции args
      Given подготовлен single-step workflow для Agent type claude
      And process Agent сначала активирует session, а на resume записывает args и завершает attempt
      When запускаются start и resume через process Agent
      Then lifecycle завершается с кодом 0
      And process Agent получил точные resume args для claude

  @process @cli:вывод-команд
  Rule: Human adapters напрямую занимают TTY и активируют session через hook
    Codex start использует `--model model --config model_reasoning_effort="high" <prompt>`, Codex resume добавляет начальный `resume` и session ID; Claude использует `--model model --effort high [--resume <session-id>] <prompt>`.

    Scenario Outline: Human start и native resume получают точные type-specific args
      Given подготовлен single-step human workflow для Agent type <type>
      And process human Agent записывает args, активирует session и на resume завершает attempt
      When human workflow запускается и продолжается через системный pseudo-terminal
      Then lifecycle завершается с кодом 0
      And обе human команды получили прямой TTY
      And process human Agent получил точные start и resume args для <type>

      Examples:
        | type   |
        | codex  |
        | claude |

  @process @cli:вывод-команд
  Rule: Обязательный session event интерпретируется fail-fast
    Codex требует `{"type":"thread.started","thread_id":"<session-id>"}`, а Claude — `{"type":"system","subtype":"init","session_id":"<session-id>"}`; повтор ID идемпотентен, другой ID противоречив, отсутствие события и невалидный JSON ошибочны, неизвестный валидный event игнорируется.

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

    Scenario Outline: Повтор одного session ID и неизвестный валидный event принимаются идемпотентно
      Given подготовлен single-step workflow для Agent type <type>
      And process Agent для <type> повторяет session ID и передаёт неизвестный валидный event
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 0
      And durable attempt содержит одну session activation

      Examples:
        | type   |
        | codex  |
        | claude |

  @process @cli:корень-состояния-и-переменные-окружения
  Rule: Executable override не выполняет PATH lookup
    Непустой ORC_AGENT_COMMAND является единым absolute executable regular file для обоих Agent types; override не добавляет type-ID и не меняет type-specific args, environment или protocol.

    Scenario Outline: Невалидный override отклоняется до запуска
      Given подготовлен single-step workflow для Agent type codex
      And ORC_AGENT_COMMAND является <case>
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 3
      And initial attempt не создан

      Examples:
        | case                           |
        | relative path                  |
        | отсутствующий absolute path    |
        | directory                      |
        | non-executable regular file    |

  @process @cli:вывод-команд
  Rule: Agent session view остаётся volatile
    Codex `item.completed` с `item.type = "agent_message"` и Claude `assistant` с text content blocks обновляют только volatile view; неизвестные валидные events игнорируются.

    Scenario Outline: TTY board нормализует сообщения встроенного Agent type
      Given подготовлен single-step workflow для Agent type <type>
      And process Agent игнорирует неизвестный event, публикует два сообщения для <type> и завершает attempt
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
      And process Agent игнорирует неизвестный event, публикует два сообщения для codex и завершает attempt
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 0
      And Agent board отсутствует в выводе
