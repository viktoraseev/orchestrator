Feature: Control endpoint активных Agent attempts
  Дочерние control-команды передают события единственному supervisor текущего run и никогда не изменяют durable-файлы напрямую.

  @process @cli:session-activate-и-attempt-complete
  Rule: Parent принимает control call только для активного inherited context
    Перед каждым Agent process supervisor передаёт endpoint, RunId и attempt через environment; parent принимает запрос только для обслуживаемого run и активного attempt, а недоступный, чужой или закрытый context возвращает код 5 без изменения durable run.

    Scenario: Дочерняя команда публикует completion только через parent supervisor
      Given подготовлен single-step workflow с output result
      And process Agent публикует artifact final через attempt complete
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 0
      And дочерний orchestrator использовал унаследованные ORC_HOME, ORC_CONTROL_ENDPOINT, ORC_RUN_ID и ORC_ATTEMPT
      And durable artifact result содержит bytes final
      And lifecycle сообщает о завершении run

    Scenario Outline: Чужой control context отклоняется пока supervisor активен
      Given подготовлен single-step workflow с output result
      And process Agent отправляет completion с чужим control context "<context>"
      When запускается orchestrator start delivery
      Then lifecycle завершается с кодом 1
      And дочерний control call завершился с кодом 5
      And чужой completion не добавил completed или artifact

      Examples:
        | context |
        | RunId   |
        | attempt |

    Scenario: Control call после возврата Agent не пишет в завершившийся run
      Given подготовлен single-step workflow без outputs
      And process Agent сохраняет control context и возвращается
      When дочерняя session activate вызывается после завершения supervisor
      Then lifecycle завершается с кодом 5
      And закрытый control endpoint удалён
      And Agent не запускался и durable run не изменился

  @process @cli:сигналы-и-закрытие-терминала
  Rule: Один supervisor использует один защищённый volatile endpoint
    Все одновременные Agent attempts run используют один Unix socket с правами 600; fail-fast одного process закрывает его control context и завершает соседние process groups, а после штатного выхода endpoint удаляется и после crash следующий supervisor удаляет stale path и создаёт новый непереиспользуемый endpoint.

    Scenario: Параллельные attempts разделяют endpoint до завершения supervisor
      Given подготовлен diamond workflow с общим лимитом 2
      And process branch Agents настроены для fail-fast
      When process fail-fast освобождает заблокированную соседнюю ветвь
      Then lifecycle завершается с кодом 1
      And обе process ветви использовали один endpoint и правая получила SIGTERM
      And общий endpoint был Unix socket с правами 600 и удалён после supervisor

    Scenario: Resume после SIGKILL удаляет stale endpoint и создаёт новый
      Given подготовлен process Agent блокирующийся после сохранения control context
      When supervisor завершается SIGKILL и run продолжается новым supervisor
      Then lifecycle завершается с кодом 0
      And stale endpoint существовал после crash, но удалён при resume
      And новый endpoint отличается от stale и удалён после supervisor
