---
id: orc-gpqe
status: closed
deps: [orc-4ddc]
links: []
created: 2026-09-01T13:15:22Z
type: feature
priority: 1
assignee: viktoraseev
tags: [run, artifact, inspection, cli, read-only, mvp]
---
# Читать durable artifact выбранного attempt

Добавить read-only команду orchestrator run artifact <run-id> <attempt-n> <input-id>, которая пишет точные bytes опубликованного artifact в stdout.

## Design

Команда разрешает attempt через проверенную durable-модель run, требует terminal completed и объявленный InputId, затем потоково копирует единственный artifact через storage boundary; текстовый presentation layer не преобразует bytes.

## Acceptance Criteria

Команда побайтово возвращает UTF-8 и бинарный artifact, включая пустой; выбирает версию строго по глобальному attempt number и не подменяет её latest-версией; неизвестные run/attempt/InputId и незавершённый attempt получают документированные категории ошибок без частичного stdout; orphan и противоречивые artifact files не читаются; read-only вызов не берёт lifecycle lock, не запускает Agent и не меняет durable state; cli.md, format.spec.md при необходимости и source-tagged API/process acceptance обновлены; canonical checks проходят.


## Notes

**2026-09-01T13:24:25Z**

Добавлен orchestrator run artifact: полная validation до открытия, точный глобальный attempt и InputId, потоковое бинарное копирование без преобразования и без stdout на validation errors.
