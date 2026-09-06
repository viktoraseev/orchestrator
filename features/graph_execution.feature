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

  @workflow:initial-activation-dependencies-и-frontier @workflow:циклы-terminal-и-blocked-run @spec:создание-и-восстановление-agent-attempt @cli:resume
  Rule: Циклический workflow повторно активирует Steps по свежим artifacts

    Scenario: Первый повторный обход использует feedback вместо bootstrap input
      Given подготовлен циклический workflow a → b → c → a
      When цикл доходит до незавершённой повторной activation a
      Then lifecycle завершается с кодом 1
      And attempts созданы как 0 a, 1 b, 2 c, 3 a
      And bootstrap a получает пустой input, а повторный a получает input 2
      And каждый завершённый Step передаёт следующему свежий artifact
      And lifecycle не сообщает о завершении run

    Scenario: Resume продолжает повторную activation и следующий обход без дубликатов
      Given подготовлен циклический workflow a → b → c → a
      When незавершённая повторная activation a продолжается через resume
      Then lifecycle завершается с кодом 1
      And resume запускает attempts 3 a, 4 b
      And следующий b получает input 3
      And attempts первого обхода и их artifacts не изменены

  @workflow:циклы-terminal-и-blocked-run @workflow:initial-activation-dependencies-и-frontier @cli:resume @cli:коды-завершения
  Rule: Частично удовлетворённая dependency group блокирует run детерминированно

    Scenario: Повторный resume сохраняет blocked run без побочных эффектов
      Given подготовлен durable run с частично удовлетворёнными dependency groups
      When blocked run дважды продолжается через lifecycle API
      Then оба resume завершаются с кодом 1 и одинаковой диагностикой
      And диагностика перечисляет отсутствующие source Steps c, b
      And Agent не запускался и durable run не изменился

    Scenario: Полная dependency group создаёт ровно один target attempt
      Given подготовлен durable run с полностью удовлетворённой dependency group
      When готовый target продолжается через lifecycle API
      Then lifecycle завершается с кодом 1
      And создан ровно один join attempt с input 1, 2

    @process
    Scenario: CLI возвращает стабильную blocked-диагностику
      Given подготовлен durable run с частично удовлетворёнными dependency groups
      And process Agent не должен запускаться
      When blocked run продолжается через CLI
      Then lifecycle завершается с кодом 1
      And stderr сообщает blocked и отсутствующие source Steps c, b

  @workflow:планирование @spec:блокировка-run @cli:коды-завершения
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
