---
id: orc-r10c
status: closed
deps: []
links: []
created: 2026-09-07T20:42:29Z
type: bug
priority: 2
assignee: viktoraseev
---
# Передавать foreground TTY human Agent process group

Human Agent запускается в отдельной process group, но supervisor не передаёт ей foreground controlling TTY; чтение stdin останавливает Agent сигналом SIGTTIN. Передавать foreground группе на время attempt и восстанавливать supervisor group после возврата.

## Acceptance Criteria

Process acceptance запускает human fake с реальным чтением TTY и подтверждает отсутствие SIGTTIN; signal shutdown сохраняет process-group semantics; незавершённый review attempt восстанавливается через resume; ./scripts/test.sh и ./scripts/build.sh проходят.


## Notes

**2026-09-07T20:54:07Z**

Human Agent больше не создаёт отдельную background process group: он наследует foreground group supervisor и может читать controlling TTY без SIGTTIN; внутренний fail-fast адресует его process, non-human Agent и Process сохраняют отдельные process groups. Acceptance fixture теперь реально читает решение accept через pseudo-terminal. Проверено: cargo test --test lifecycle, ./scripts/test.sh, ./scripts/build.sh и git diff --check — успешно.
