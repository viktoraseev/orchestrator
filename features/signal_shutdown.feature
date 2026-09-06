Feature: Signal shutdown lifecycle
  Первый termination signal задаёт итог команды и управляет всеми supervised process groups.

  @process @cli:сигналы-и-закрытие-терминала @cli:коды-завершения
  Rule: Первый termination signal штатно завершает supervisor

    Scenario Outline: Supervisor пересылает signal Agent process group
      Given подготовлен single-step workflow без outputs
      And process Agent ожидает termination signal
      When supervisor получает <signal>
      Then lifecycle завершается с кодом <code>
      And Agent process group получила тот же signal
      And последняя lifecycle строка сообщает exited

      Examples:
        | signal  | code |
        | SIGHUP  | 129  |
        | SIGINT  | 130  |
        | SIGTERM | 143  |

    Scenario: Второй termination signal немедленно эскалирует shutdown
      Given подготовлен single-step workflow без outputs
      And process Agent игнорирует первый SIGTERM
      When supervisor получает второй SIGTERM
      Then shutdown эскалирован до SIGKILL без изменения кода
      And последняя lifecycle строка сообщает exited
      And initial attempt остаётся незавершённым
      When run после signal shutdown продолжается и завершается
      Then lifecycle завершается с кодом 0
      And attempt завершён terminal event completed

    @slow
    Scenario: Один termination signal эскалирует shutdown через 10 секунд
      Given подготовлен single-step workflow без outputs
      And process Agent игнорирует первый SIGTERM
      When после одного SIGTERM истекает константный shutdown deadline
      Then shutdown эскалирован до SIGKILL без изменения кода
      And эскалация произошла не раньше 10 секунд
      And последняя lifecycle строка сообщает exited
