# Workflow graph: спецификация

Этот документ определяет оставшуюся семантику cycles, terminal и blocked run, planning и validation workflow graph. Модель graph, activations, frontier и inputs определена в `features/*.feature`, формат workflow-файла и описание его полей — в `format.spec.md`, а публичное поведение CLI — в `cli.md`.

## Циклы, terminal и blocked run

Граф может содержать циклы. Повторное достижение Step создаёт новый attempt; предыдущие attempts и artifacts не переиспользуются и не изменяются.

В цикле `a → b → c → a`, где `a` описан первым, initial attempt `a` получает пустой input. Завершение `a` активирует `b`, завершение `b` — `c`, а свежий завершённый attempt `c` удовлетворяет `depends-on` и создаёт следующий attempt `a`. Пустой bootstrap-input не участвует в следующих итерациях.

У цикла могут быть независимые входные, feedback- и выходные artifacts. Один Step может зависеть одновременно от участника цикла и внешнего Step, а его outputs могут передаваться Steps внутри и вне цикла. Специального общего cycle input или output нет.

Step terminal, если ни один Step не содержит его ID в `depends-on`. Run завершён, когда отсутствуют запущенные и незавершённые attempts, ready activations и частично удовлетворённые dependencies. Dependency group частично удовлетворена, если для target Step свеж хотя бы один, но не все source attempts; тогда run blocked, а не completed.

## Планирование

В одном run могут одновременно существовать attempts разных Steps; положительный `max-parallel-agents` из materialized workflow ограничивает общее число одновременно работающих Agent и Process executors, включая human и процессы native resume.

На scheduling pass запускаемой работой являются ready activations и незавершённые attempts, которые текущая команда ещё не запускала; возврат процесса без completion не ставит тот же attempt в запускаемую работу повторно до следующего явного `resume` по Rule «Start обещает только durable run» в `features/lifecycle.feature`.

- На каждом scheduling pass supervisor сначала учитывает уже работающие процессы; если свободных слотов нет, ready activations остаются во frontier, а незапущенные unfinished attempts ожидают слота внутри текущей команды.
- Если human attempt не работает, есть запускаемая human-работа и свободен хотя бы один слот, но у lifecycle-команды нет TTY, supervisor не запускает никакую новую работу этого scheduling pass и завершает команду runtime fail-fast с кодом `1`; headless-запуск human attempt запрещён.
- Одновременно выполняется не более одного human attempt. Если human attempt не работает, есть запускаемая human-работа, свободен хотя бы один слот и доступен TTY, supervisor первым выбирает работу Step, раньше описанного в materialized workflow; на время работы его процесс эксклюзивно занимает TTY команды, а остальная human-работа ожидает.
- Оставшиеся свободные слоты заполняются запускаемой non-human-работой в порядке materialized workflow; не выбранная из-за лимита работа ожидает следующего scheduling pass.
- Non-human attempts работают параллельно с human attempt без доступа к TTY и без вывода сырого live-потока в терминал. Выбранные новые activations одного scheduling pass сортируются по порядку Steps; native resume сохраняет прежний `n`.
- После durable-изменения, влияющего на frontier или незавершённые attempts, выполняется следующий scheduling pass.
- После `/exit` human attempt supervisor входит в user shutdown и больше не запускает работу из frontier или незавершённых attempts; возвраты уже работающих non-human процессов по-прежнему изменяют durable-модель, но появившаяся вследствие них ready-работа остаётся для следующего явного `resume`.

## Validation

`validate` и preflight `start` независимо от способа выбора полностью materialize’ят в памяти и проверяют кандидат выбранного workflow:

- соответствие workflow и prompt templates `format.spec.md`;
- корректность ParameterIds, Process executable, cwd, args, stdout и ссылок Process placeholders;
- существование явного Agent каждого Step либо `default-agent`, валидность его type, model и reasoning и поддержку native resume Agent type;
- существование всех StepIds из `depends-on` и всех PromptIds;
- отсутствие любых placeholders в prompt template первого описанного Step;
- что каждый placeholder prompt ссылается на Step из `depends-on` и InputId из `outputs` этого source Step;
- статическую достижимость: первый Step достижим initial activation, а другой Step — только если все его dependencies достижимы; non-entry Step с пустым `depends-on` не имеет activation path и невалиден;
- допустимость cycles, включая `depends-on` первого Step для повторной activation; циклическая компонента без bootstrap-пути недостижима;
- допустимость outputs без consumers и отсутствия terminal Step.

Preflight не записывает snapshot в run: `validate` завершает работу после проверки кандидата, а `start` durable-публикует его только после резервирования run по `format.spec.md`. Во время выполнения статическая reachability повторно не вычисляется.

- `workflow plan` возвращает validated кандидат с объявлениями parameters, effective Agent или Process executor, prompt content и parallel limit через ту же validation boundary, что `start`, но без runtime parameter values, резервирования или публикации run.

## Fail-fast input validation

При создании и восстановлении attempt input обязан содержать ровно один source attempt для каждого Step в порядке его `depends-on`. Каждый source attempt обязан существовать, быть успешно завершённым, иметь меньший номер и быть свежее нижней границы target Step. Для каждого InputId из его `outputs` обязан существовать соответствующий artifact.

Неполная, старая или противоречивая input group является ошибкой fail-fast. После публикации attempt выбранные source numbers неизменяемы.
