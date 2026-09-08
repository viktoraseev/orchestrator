Feature: Условные ветви и последовательные повторения участка графа
  Rule: Completion выбирает допустимый полный набор outputs
    Верхняя sequence является all; вложенные непустые all и one-of требуют соответственно все элементы или ровно одну целую ветвь, OutputIds уникальны во всём выражении, а необъявленные artifacts запрещены.

    Scenario Outline: Вложенная output group проверяется целиком
      Given outputs первого Step равны "[report, {one-of: [{all: [fix, patch]}, done]}]"
      When Agent предлагает artifacts "<artifacts>"
      Then completion принят "<accepted>"
      And опубликованы artifacts "<published>"
      Examples:
        | artifacts             | accepted | published         |
        | report,fix,patch      | yes      | report,fix,patch  |
        | report,done           | yes      | report,done       |
        | report,fix            | no       |                   |
        | report                | no       |                   |
        | report,fix,patch,done | no       |                   |
        | report,done,extra     | no       |                   |

  Rule: Qualified dependencies выбирают artifacts одной завершённой source version
    Depends-on поддерживает StepId, mapping step/output и рекурсивные all/one-of; target получает все опубликованные artifacts выбранных sources, каждый source представлен одним attempt в порядке первого выбранного листа.
    One-of выбирает удовлетворённую ветвь с максимальным source number, при равенстве — первую объявленную; inputs фиксируются при создании attempt и повторно не вычисляются на той же комбинации.

    Scenario: Условный diamond закрывает невыбранную ветвь
      Given подготовлен условный diamond с выбором done
      When условный workflow выполняется до завершения
      Then выполнены Steps "review,finish,collect"
      And результат run равен "completed"
      And attempt "finish" получает inputs "0"
      And attempt "collect" получает inputs "1"

    Scenario: Несколько ссылок на source передают все его artifacts один раз
      Given подготовлен workflow с двумя qualified ссылками на source
      When условный workflow выполняется до завершения
      Then выполнены Steps "source,target"
      And attempt "target" получает inputs "0"
      And target получает artifacts "source:report,source:fix,source:patch"

    Scenario: One-of выбирает самую позднюю завершённую ветвь
      Given подготовлен diamond с one-of на join
      When условный workflow выполняется до завершения
      Then выполнены Steps "root,left,right,join"
      And attempt "join" получает inputs "2"

    Scenario: Равенство source numbers разрешается порядком объявления
      Given подготовлен one-of с равными максимальными source numbers
      When условный workflow выполняется до завершения
      Then attempt "join" получает inputs "0,2"

  @cli:resume
  Rule: Повторяемый участок имеет один вход и одну точку решения после всех выбранных ветвей
    Допускается не более одного циклического участка; его вход — первый описанный Step участка, все обратные зависимости входа внутри участка ведут из единственного решающего Step, а после удаления этого ребра участок является DAG.
    Выходящие из участка зависимости разрешены только от решающего Step; решающий Step ожидает завершения либо закрытия всех выбранных ветвей участка, ранние возвраты, вложенные и независимые циклы отклоняются.
    Внутренние inputs относятся к текущему выполнению участка, внешние результаты переиспользуются; новый вход создаётся только после решения предыдущего выполнения и не использует старую команду из истории, если последняя source version выбрала другую ветвь.
    Start сохраняет bootstrap первого Step с пустым input; вход участка после внешнего контекста может выбирать между начальным внешним source и обратной связью, но уже использованная начальная ветвь не активирует его снова.

    Scenario: Цикл повторяет обе ветви и переиспользует внешний контекст
      Given подготовлен условный цикл с постоянным контекстом
      When условный workflow выполняется до завершения
      Then выполнены Steps "context,a,left,right,check,a,left,right,check,finish"
      And результат run равен "completed"
      And attempts "a" получают input groups "0;0,4"
      And attempts "check" получают input groups "2,3;6,7"

    Scenario: Закрытая ветвь внутри цикла не задерживает решение
      Given подготовлен цикл с условной внутренней ветвью
      When условный workflow выполняется до завершения
      Then выполнены Steps "a,right,check,a,right,check,finish"
      And результат run равен "completed"

    Scenario: Решение ожидает длинную выбранную ветвь даже при one-of
      Given подготовлен цикл с короткой и длинной ветвями
      When условный workflow выполняется до завершения
      Then выполнены Steps "a,left,right,tail,check,finish"
      And attempt "check" получает inputs "3"
      And результат run равен "completed"

    Scenario: Resume сохраняет выбранные inputs и завершает цикл
      Given подготовлен условный цикл с постоянным контекстом
      When выполнение прерывается на повторном a и продолжается через resume
      Then выполнены Steps "context,a,left,right,check,a,a,left,right,check,finish"
      And attempts "a" получают input groups "0;0,4"
      And результат run равен "completed"

    Scenario: Resume отклоняет решение опубликованное до завершения длинной ветви
      Given подготовлен цикл с короткой и длинной ветвями
      When в незавершённый run добавлено преждевременное решение
      Then resume отклоняет run кодом 3 без запуска Agent

  Rule: Validation учитывает выражения и гарантии обязательных placeholders
    Пустые группы, повторные OutputIds, неизвестные artifact references, невыполнимые согласованные группы и обязательные placeholders без гарантированного artifact отклоняются до создания run; поле fresh не поддерживается.

    Scenario Outline: Некорректный условный workflow отклоняется до запуска
      Given подготовлен невалидный условный workflow "<case>"
      When условный workflow выполняется до завершения
      Then условный lifecycle возвращает код 3 без run
      Examples:
        | case                         |
        | пустая группа                |
        | повторный output             |
        | неизвестный output           |
        | несовместимые outputs source |
        | несовместимые ветви          |
        | негарантированный placeholder |
        | ранний возврат               |
        | два независимых цикла        |
        | вложенный цикл               |
        | поле fresh                   |

  Rule: Source show и plan сохраняют выражения в публичном выводе
    Text сохраняет прежнюю форму плоских списков, печатает группы как all(...)/one-of(...) и qualified leaves как step:output; JSON сохраняет рекурсивную структуру выражений и mapping step/output.

    Scenario: Условия видны до запуска в source и materialized представлении
      Given подготовлен workflow с двумя qualified ссылками на source
      When проверяется source и materialized представление условий
      Then source show text содержит `outputs=report,one-of(all(fix,patch),done)` и `depends-on=source:fix,source:report`
      And source show JSON сохраняет nested one-of/all outputs report, fix, patch, done и qualified dependencies source:fix и source:report в mappings step/output
      And materialized plan text содержит `outputs=report,one-of(all(fix,patch),done)` и `depends-on=source:fix,source:report`
      And materialized plan JSON сохраняет nested one-of/all outputs report, fix, patch, done и qualified dependencies source:fix и source:report в mappings step/output
      And run ещё не создан

  Rule: Inspection перечисляет только artifacts выбранной output-ветви
    Artifact из невыбранной ветви завершённого attempt отсутствует и возвращает код 4; остальные опубликованные artifacts остаются доступными после resume.

    Scenario: Закрытый fix отсутствует в descriptors и отдельном чтении
      Given подготовлен условный diamond с выбором done
      When условный workflow выполняется до завершения
      Then inspection видит done и не видит fix
