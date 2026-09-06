Feature: Human Agent lifecycle
  Human attempt использует только прямой TTY lifecycle-команды.

  @workflow:планирование @spec:сущности @cli:вывод-команд @cli:коды-завершения @cli:сигналы-и-закрытие-терминала
  Rule: Human attempt запускается только с доступным TTY
    Human Agent process напрямую и эксклюзивно занимает TTY lifecycle-команды; если human attempt становится runnable без TTY, lifecycle завершается runtime failure с кодом 1 до запуска Agent, а headless-режима нет.

    Scenario: Headless lifecycle не создаёт human Agent process
      Given подготовлен single-step human workflow
      When human workflow запускается без TTY
      Then lifecycle завершается с кодом 1
      And human Agent не запускался

    Scenario: API driver передаёт human Agent прямой terminal mode
      Given подготовлен single-step human workflow
      When human workflow запускается с доступным TTY
      Then lifecycle завершается с кодом 0
      And Agent получил human terminal mode

    @process
    Scenario: Human process напрямую наследует TTY lifecycle-команды
      Given подготовлен single-step human workflow
      And process human Agent проверяет stdin и stdout TTY
      When human workflow запускается через системный pseudo-terminal
      Then lifecycle завершается с кодом 0
      And process human Agent подтвердил прямой TTY

  @cli:/exit-и-user-shutdown @cli:вывод-команд @workflow:планирование @spec:создание-и-восстановление-agent-attempt
  Rule: Явный /exit оставляет human attempt для resume
    Во время user shutdown возврат одного уже работающего non-human Agent не прерывает ожидание остальных, но новые activations не запускаются.

    Scenario: User shutdown завершается успешно и resume продолжает session
      Given подготовлен single-step human workflow
      When human Agent активирует session human-session и выполняет /exit
      Then lifecycle завершается с кодом 0
      And lifecycle сообщает interrupted и команду resume
      And human attempt остаётся незавершённым
      When human run продолжается с TTY и завершается
      Then lifecycle завершается с кодом 0
      And resume продолжил attempt 0 с session human-session

    Scenario: User shutdown дожидается уже работающего non-human completion
      Given подготовлен workflow с human и non-human ветвями
      When human ветвь выполняет /exit одновременно с completion worker
      Then lifecycle завершается с кодом 0
      And worker completion durable, а следующая activation не создана
