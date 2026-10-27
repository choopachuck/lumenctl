"""MQTT topic and payload parsing for lumenctl.

This module does not import Home Assistant. The integration platforms only
display the model built here.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from enum import Enum
from typing import Any

DEFAULT_BASE_TOPIC = "lumenctl"

NUMBER_KINDS = ("contrast", "volume", "red", "green", "blue")
SELECT_KINDS = {
    "input": "inputs",
    "color_preset": "color_presets",
    "display_mode": "display_modes",
}
SELECT_CURRENT = {
    "input": "input",
    "color_preset": "color_preset",
    "display_mode": "display_mode",
}
_DISABLED_BY_DEFAULT = frozenset({"red", "green", "blue"})
_POWERS = frozenset({"on", "standby", "off"})


class TopicKind(Enum):
    """Which retained lumenctl topic a message belongs to."""

    STATUS = "status"
    HUB_INFO = "hub_info"
    MONITOR_INFO = "monitor_info"


@dataclass(frozen=True)
class ParsedTopic:
    """A lumenctl topic addressed to one computer, and maybe one monitor."""

    kind: TopicKind
    node_id: str
    monitor_key: str | None = None


@dataclass
class RangeValue:
    """A slider value already clamped so min <= current <= max."""

    current: int
    min: int
    max: int


@dataclass
class HubInfo:
    """Retained description of one lumenctl process."""

    node_id: str
    name: str
    sw_version: str | None


@dataclass
class MonitorInfo:
    """Retained description of one monitor. The key comes from the topic."""

    key: str
    id: str | None
    name: str
    manufacturer: str | None
    model: str | None
    serial_number: str | None
    ddc_ok: bool
    power: str
    brightness: RangeValue | None
    contrast: RangeValue | None
    volume: RangeValue | None
    red: RangeValue | None
    green: RangeValue | None
    blue: RangeValue | None
    input: str | None
    inputs: list[str]
    color_preset: str | None
    color_presets: list[str]
    display_mode: str | None
    display_modes: list[str]


def normalize_base_topic(topic: str) -> str:
    """Strip surrounding whitespace and slashes from a base topic."""
    return topic.strip().strip("/")


def normalize_node_filter(node: str | None) -> str | None:
    """Empty text means every computer under the base topic."""
    if node is None:
        return None
    text = node.strip()
    return text or None


def checked_base_topic(topic: str) -> str:
    """Return a base topic or raise ValueError('invalid_base_topic')."""
    base = normalize_base_topic(topic)
    if not base or any(char in base for char in "+# \t"):
        raise ValueError("invalid_base_topic")
    return base


def checked_node_filter(node: str) -> str | None:
    """Return a node id, None when empty, or raise ValueError('invalid_node_id')."""
    text = normalize_node_filter(node)
    if text is None:
        return None
    if any(char in text for char in "+#/ \t"):
        raise ValueError("invalid_node_id")
    return text


def merge_settings(
    data: dict[str, Any],
    options: dict[str, Any] | None = None,
    *,
    base_key: str = "base_topic",
    node_key: str = "node_id",
    default_base: str = DEFAULT_BASE_TOPIC,
) -> tuple[str, str | None]:
    """Options override the original config entry data."""
    merged = dict(data)
    if options:
        merged.update(options)
    raw_base = merged.get(base_key)
    base = normalize_base_topic(raw_base if isinstance(raw_base, str) else default_base)
    raw_node = merged.get(node_key)
    return base or default_base, normalize_node_filter(raw_node if isinstance(raw_node, str) else None)


def subscription_topics(base_topic: str) -> tuple[str, str, str]:
    """Status, hub info, and per-monitor info subscriptions."""
    root = normalize_base_topic(base_topic)
    return (f"{root}/+/status", f"{root}/+/info", f"{root}/+/+/info")


def parse_topic(base_topic: str, topic: str, node_filter: str | None = None) -> ParsedTopic | None:
    """Classify a topic. State and command topics are ignored on purpose."""
    root = normalize_base_topic(base_topic)
    if not root:
        return None
    prefix = f"{root}/"
    if not topic.startswith(prefix):
        return None
    parts = topic[len(prefix) :].split("/")
    monitor_key: str | None = None
    if len(parts) == 2 and parts[0] and parts[1] == "status":
        kind = TopicKind.STATUS
        node_id = parts[0]
    elif len(parts) == 2 and parts[0] and parts[1] == "info":
        kind = TopicKind.HUB_INFO
        node_id = parts[0]
    elif len(parts) == 3 and parts[0] and parts[1] and parts[2] == "info":
        kind = TopicKind.MONITOR_INFO
        node_id = parts[0]
        monitor_key = parts[1]
    else:
        return None
    wanted = normalize_node_filter(node_filter)
    if wanted is not None and node_id != wanted:
        return None
    return ParsedTopic(kind=kind, node_id=node_id, monitor_key=monitor_key)


def hub_topic(base_topic: str, node_id: str, leaf: str) -> str:
    return f"{normalize_base_topic(base_topic)}/{node_id}/{leaf}"


def monitor_set_topic(base_topic: str, node_id: str, key: str, field: str) -> str:
    return f"{normalize_base_topic(base_topic)}/{node_id}/{key}/{field}/set"


def light_command(state: str, brightness: int | None = None) -> str:
    """JSON light schema. Brightness is the monitor value, not 0–255."""
    body: dict[str, object] = {"state": state}
    if brightness is not None:
        body["brightness"] = brightness
    return json.dumps(body, separators=(",", ":"))


def hub_identifier(node_id: str) -> str:
    return f"lumenctl_{node_id}"


def monitor_identifier(node_id: str, key: str) -> str:
    return f"lumenctl_{node_id}_{key}"


def entity_unique_id(node_id: str, kind: str, key: str | None = None) -> str:
    if key:
        return f"lumenctl_{node_id}_{key}_{kind}"
    return f"lumenctl_{node_id}_{kind}"


def platform_for(kind: str) -> str:
    """Home Assistant platform domain that owns this entity kind."""
    if kind == "light":
        return "light"
    if kind in NUMBER_KINDS:
        return "number"
    if kind in SELECT_KINDS:
        return "select"
    if kind == "ddc":
        return "binary_sensor"
    if kind == "command":
        return "text"
    if kind == "rescan":
        return "button"
    raise KeyError(kind)


def enabled_by_default(kind: str) -> bool:
    """RGB sliders stay in the registry but are hidden until enabled."""
    return kind not in _DISABLED_BY_DEFAULT


def entity_kinds(info: MonitorInfo) -> set[str]:
    """Entity kinds this monitor description should currently expose."""
    kinds = {"ddc"}
    if info.brightness is not None:
        kinds.add("light")
    for field in NUMBER_KINDS:
        if getattr(info, field) is not None:
            kinds.add(field)
    if info.inputs:
        kinds.add("input")
    if info.color_presets:
        kinds.add("color_preset")
    if info.display_modes:
        kinds.add("display_mode")
    return kinds


def range_to_ha_brightness(value: RangeValue) -> int:
    """Map a monitor range onto the 0–255 brightness Home Assistant stores."""
    span = value.max - value.min
    if span <= 0:
        return 255 if value.current > 0 else 0
    ratio = (value.current - value.min) / span
    return max(0, min(255, round(ratio * 255)))


def ha_brightness_to_native(value: RangeValue, brightness: int) -> int:
    """Map a Home Assistant brightness back onto the monitor range."""
    span = value.max - value.min
    if span <= 0:
        return value.min
    native = round(value.min + (brightness / 255) * span)
    return max(value.min, min(value.max, native))


def parse_hub_info(payload: str, node_id: str) -> HubInfo:
    data = json.loads(payload)
    if not isinstance(data, dict):
        raise ValueError("hub info must be a JSON object")
    name = data.get("name")
    if not isinstance(name, str) or not name.strip():
        name = f"lumenctl ({node_id})"
    version = data.get("sw_version")
    if version is not None and not isinstance(version, str):
        raise ValueError("sw_version")
    return HubInfo(node_id=node_id, name=name.strip(), sw_version=version or None)


def decode_monitor_payload(payload: str, key: str) -> MonitorInfo | None:
    """Parse monitor info. An empty payload means the monitor was removed."""
    if payload.strip() == "":
        return None
    data = json.loads(payload)
    if not isinstance(data, dict):
        raise ValueError("monitor info must be a JSON object")
    power = data.get("power", "on")
    if power not in _POWERS:
        raise ValueError("power")
    ddc_ok = data.get("ddc_ok", False)
    if not isinstance(ddc_ok, bool):
        raise ValueError("ddc_ok")
    return MonitorInfo(
        key=key,
        id=_opt_str(data.get("id"), "id"),
        name=_opt_str(data.get("name"), "name") or key,
        manufacturer=_opt_str(data.get("manufacturer"), "manufacturer"),
        model=_opt_str(data.get("model"), "model"),
        serial_number=_opt_str(data.get("serial_number"), "serial_number"),
        ddc_ok=ddc_ok,
        power=power,
        brightness=_range(data.get("brightness"), "brightness"),
        contrast=_range(data.get("contrast"), "contrast"),
        volume=_range(data.get("volume"), "volume"),
        red=_range(data.get("red"), "red"),
        green=_range(data.get("green"), "green"),
        blue=_range(data.get("blue"), "blue"),
        input=_opt_str(data.get("input"), "input"),
        inputs=_names(data.get("inputs"), "inputs"),
        color_preset=_opt_str(data.get("color_preset"), "color_preset"),
        color_presets=_names(data.get("color_presets"), "color_presets"),
        display_mode=_opt_str(data.get("display_mode"), "display_mode"),
        display_modes=_names(data.get("display_modes"), "display_modes"),
    )


def _as_int(value: object, field: str) -> int:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(field)
    return int(round(value))


def _range(raw: object, field: str) -> RangeValue | None:
    if raw is None:
        return None
    if not isinstance(raw, dict):
        raise ValueError(field)
    current = _as_int(raw.get("current"), field)
    minimum = _as_int(raw.get("min", 0), field)
    maximum = _as_int(raw.get("max", 100), field)
    if maximum < minimum:
        maximum = minimum
    if current < minimum:
        minimum = current
    if current > maximum:
        maximum = current
    return RangeValue(current=current, min=minimum, max=maximum)


def _opt_str(raw: object, field: str) -> str | None:
    if raw is None:
        return None
    if not isinstance(raw, str):
        raise ValueError(field)
    text = raw.strip()
    return text or None


def _names(raw: object, field: str) -> list[str]:
    if raw is None:
        return []
    if not isinstance(raw, list):
        raise ValueError(field)
    names: list[str] = []
    for item in raw:
        if not isinstance(item, str) or not item:
            raise ValueError(field)
        names.append(item)
    return names
