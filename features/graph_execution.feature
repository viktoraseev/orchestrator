Feature: Выполнение workflow graph
  Durable facts определяют input mapping, frontier и глобальную нумерацию attempts.

  @workflow:initial-activation-dependencies-и-frontier @workflow:inputs-и-prompt @workflow:fail-fast-input-validation @spec:создание-и-восстановление-agent-attempt @format:agent-attempt-record
  Rule: Линейный target получает зафиксированную версию source artifacts

    Scenario: Target получает path, content и durable input source attempt
      Given подготовлен линейный workflow source → target
      When source публикует result hello, а target завершается
      Then lifecycle завершается с кодом 0
      And target attempt 1 имеет input 0
      And target Agent получает artifact source:result и prompt с path и content hello
      And lifecycle сообщает о завершении run

    Scenario: Невалидный UTF-8 content не создаёт target attempt
      Given подготовлен линейный workflow source → target
      When source публикует невалидный UTF-8 result
      Then lifecycle завершается с кодом 3
      And target attempt 1 не создан

  @workflow:модель-graph @workflow:initial-activation-dependencies-и-frontier @workflow:циклы-terminal-и-blocked-run @workflow:планирование @cli:resume
  Rule: Fan-out и fan-in frontier вычисляется из durable attempts

    Scenario: Diamond graph создаёт fan-out в порядке Steps и затем fan-in
      Given подготовлен diamond workflow
      When все четыре Steps успешно завершаются
      Then lifecycle завершается с кодом 0
      And attempts созданы как 0 root, 1 left, 2 right, 3 join
      And join attempt имеет input 1, 2

    Scenario: Одинаковые InputId разных source Steps остаются разными inputs
      Given подготовлен fan-in workflow с одинаковым output shared
      When все четыре Steps публикуют свои outputs и завершаются
      Then lifecycle завершается с кодом 0
      And join Agent получает inputs left:shared и right:shared

  @workflow:планирование @spec:control-endpoint-и-события @spec:блокировка-run @cli:коды-завершения
  Rule: Non-human attempts выполняются параллельно под общим лимитом

    Scenario: Независимые ветви одновременно занимают два разрешённых slot
      Given подготовлен diamond workflow с общим лимитом 2
      When ветви выполняются через синхронизируемые fake Agents
      Then lifecycle завершается с кодом 0
      And attempts созданы как 0 root, 1 left, 2 right, 3 join
      And одновременно работали ровно 2 branch Agents
      And parallel attempts сохранили независимые session contexts

    Scenario: Ошибка одной ветви не теряет completion уже вернувшегося соседа
      Given подготовлен diamond workflow с общим лимитом 2
      When левая ветвь fail-fast завершается с ошибкой
      Then lifecycle завершается с кодом 1
      And успешная правая ветвь durable завершена, а join не создан

    @process @cli:сигналы-и-закрытие-терминала
    Scenario: Process fail-fast посылает SIGTERM соседней process group через общий supervisor
      Given подготовлен diamond workflow с общим лимитом 2
      And process branch Agents настроены для fail-fast
      When process fail-fast освобождает заблокированную соседнюю ветвь
      Then lifecycle завершается с кодом 1
      And обе process ветви использовали один endpoint и правая получила SIGTERM
