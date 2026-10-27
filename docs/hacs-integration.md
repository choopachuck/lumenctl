# Интеграция lumenctl для Home Assistant (HACS, MQTT)

Кастомная интеграция слушает брокер, уже настроенный в Home Assistant, и создаёт устройства и сущности сама. Свой MQTT-клиент она не открывает. Код лежит в [`custom_components/lumenctl`](../custom_components/lumenctl), ставится через HACS из этого репозитория (`hacs.json`, тип Integration).

Встроенный MQTT discovery остаётся для тех, кто интеграцию не ставит. Одновременно с ней его нужно выключить: уникальные id живут в разных доменах и не сольются, сущности задвоятся.

## Зачем отдельный топик `info`

Retained `{base}/{node}/{key}/state` содержит только текущие числа и имена (`brightness: 40`, `input: "HDMI1"`). Списки входов и пресетов, диапазоны слайдеров, производитель, модель и серийник уходят только в JSON discovery. Интеграция discovery не разбирает.

Командные топики не меняются. Топик `state` тоже остаётся: его продолжает читать MQTT discovery.

## Топики

`{base}` — это `{base_topic}/{node_id}`. По умолчанию `base_topic` равен `lumenctl`. `node_id` — имя компьютера или значение `mqtt.node_id` из настроек lumenctl.

| Топик | Кто пишет | Смысл |
|---|---|---|
| `{base}/status` | lumenctl, retained, LWT | `online` или `offline`. Доступность всех сущностей этого ПК |
| `{base}/info` | lumenctl, retained | JSON хаба: `node_id`, `name`, `sw_version` |
| `{base}/{key}/info` | lumenctl, retained | JSON монитора. Пустой payload удаляет монитор |
| `{base}/{key}/state` | lumenctl, retained | прежний плоский JSON для MQTT discovery |
| `{base}/{key}/light/set` | интеграция | JSON световой схемы: `{"state":"ON"}`, `{"state":"OFF"}`, `{"state":"ON","brightness":40}` |
| `{base}/{key}/{field}/set` | интеграция | `contrast`, `volume`, `red`, `green`, `blue` — целое число текстом; `input`, `color_preset`, `display_mode` — имя |
| `{base}/command` | интеграция | сырая командная строка, например `b 30` или `v 20` |
| `{base}/rescan` | интеграция | любой payload, мониторы перечитываются |

Яркость в `light/set` — абсолютное значение VCP в шкале монитора, не 0–255. Интеграция пересчитывает ползунок Home Assistant (0–255) в диапазон `min`…`max` из `info`.

`power` в `info`: `on`, `standby` или `off`. Если питание неизвестно, lumenctl публикует `on`. Для света `on` — включено, `standby` и `off` — выключено.

Числовое поле — объект или `null`:

```json
{"current": 40, "min": 0, "max": 100}
```

`min` берётся из `min_brightness` / `min_contrast` монитора, иначе 0. `max` — из `max_brightness` / `max_contrast`, иначе максимум VCP (если монитор сообщил 0, используется 100). Если текущее значение оказалось вне этого окна, границы раздвигаются до него, чтобы слайдер Home Assistant принял состояние.

`null` или пустой список значит, что сущности нет. Свет создаётся только когда `brightness` не `null`.

Пример `{base}/{key}/info`:

```json
{
  "key": "hwv6e22_5",
  "id": "HWV6E22#5&…",
  "name": "MateView",
  "manufacturer": "HWV",
  "model": "MateView",
  "serial_number": "ABC",
  "ddc_ok": true,
  "power": "on",
  "brightness": {"current": 40, "min": 0, "max": 100},
  "contrast": {"current": 50, "min": 0, "max": 100},
  "volume": null,
  "red": null,
  "green": null,
  "blue": null,
  "input": "HDMI1",
  "inputs": ["HDMI1", "DisplayPort1"],
  "color_preset": "6500K",
  "color_presets": ["6500K", "User1"],
  "display_mode": null,
  "display_modes": []
}
```

Пример `{base}/info`:

```json
{"node_id": "desktop", "name": "lumenctl (desktop)", "sw_version": "1.0.0"}
```

Идентификатор монитора в интеграции берётся из уровня топика `{key}`, не из поля JSON.

Когда монитор исчезает, lumenctl публикует пустой retained payload и в `{key}/info`, и в `{key}/state`, и снимает его discovery-конфиги.

## Настройка lumenctl

В `config.toml`, секция `mqtt`:

- `enabled` — мост в целом
- `discovery` — публиковать конфиги MQTT discovery. По умолчанию `true`, в том числе если поле отсутствует в старом файле
- `discovery = false` — конфиги discovery не публикуются (на уже объявленные топики уходит пустой retained payload, чтобы Home Assistant их снял), подписка на `{discovery_prefix}/status` не открывается. `info`, `state`, `status` и команды работают как раньше

В окне настроек это чекбокс **Publish Home Assistant MQTT discovery**. Для HACS его нужно снять.

## Поведение интеграции

Один config entry. Если MQTT в Home Assistant не настроен, поток настройки прерывается.

Поля потока:

- базовый топик, по умолчанию `lumenctl` (без пробелов и без `+`, `#`)
- идентификатор узла, необязательно. Пусто — все ПК под этим топиком. Иначе только один `node_id`

Подписки: `{base}/+/status`, `{base}/+/info`, `{base}/+/+/info`.

Устройство-хаб, идентификатор `lumenctl_{node}`:

- `text` Command → `{base}/command`. Состояние — последняя отправленная строка этой сессии
- `button` Rescan monitors → `{base}/rescan`

Устройство монитора, идентификатор `lumenctl_{node}_{key}`, `via_device` — хаб:

- `light` — имя сущности совпадает с именем монитора. Питание и яркость
- `number` contrast, volume, red, green, blue — слайдер. Красный, зелёный и синий выключены в реестре по умолчанию
- `select` input, color preset, display mode — опции из `info`, сущность появляется, когда список непустой
- `binary_sensor` DDC/CI (`ddc_ok`), диагностический, класс connectivity

Поле, которого не было в первом `info`, добавляет сущность, когда появляется (например, capabilities дочитались позже). Пропавшее поле сущность снимает. Пустой `info` удаляет устройство монитора. Офлайн хаба (`status` = `offline` или LWT) делает сущности недоступными, но не удаляет их.

Уникальные id: `lumenctl_{node}_{key}_light`, `lumenctl_{node}_{key}_{field}`, `lumenctl_{node}_{key}_ddc`, `lumenctl_{node}_command`, `lumenctl_{node}_rescan`.

Системная громкость Windows отдельной сущностью не выносится. Её можно задать командой `v 30`.

## Чего интеграция не делает

Не ходит в HTTP API lumenctl, не открывает отдельное соединение с брокером и не разбирает топики `homeassistant/...`. Системную громкость, сырой VCP и поворот экрана наружу не выводит: это по-прежнему командная строка.

## Проверка в Home Assistant

`cargo test -p lumen-core -p lumen-server` проверяет сборку `info` и флаг discovery. `pytest` из корня репозитория гоняет `tests/test_hacs_codec.py` и не требует установленный Home Assistant. Сам Home Assistant этими тестами не запускается. Ручной проход:

1. В Home Assistant настроен MQTT. В lumenctl включён MQTT, discovery выключен, базовый топик `lumenctl`.
2. Интеграция добавлена через HACS и Settings → Devices & services. Появились устройство ПК и устройство на каждый монитор.
3. Слайдер света меняет яркость. Выключение переводит монитор в power off, включение возвращает `on`.
4. Контраст, вход и цветовой пресет применяются и обновляются, если монитор меняют с трея.
5. Rescan обновляет список. Команда `b 20` меняет яркость монитора по умолчанию.
6. Выход из lumenctl переводит сущности в unavailable. Повторный запуск возвращает их без дублей.
7. Монитор, пропавший после rescan, исчезает вместе с устройством.
8. Discovery, оставленный включённым рядом с интеграцией, создаёт вторую копию сущностей. Для HACS он должен быть выключен.
