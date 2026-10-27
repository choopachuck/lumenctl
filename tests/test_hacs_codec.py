"""Parser tests. They load codec.py directly so Home Assistant is not required."""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1] / "custom_components" / "lumenctl"
spec = importlib.util.spec_from_file_location("lumenctl_codec", ROOT / "codec.py")
assert spec is not None and spec.loader is not None
codec = importlib.util.module_from_spec(spec)
sys.modules["lumenctl_codec"] = codec
spec.loader.exec_module(codec)


MONITOR = """
{
  "key": "ignored",
  "id": "HWV6E22#5",
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
"""


def test_parse_topic_classifies_retained_messages():
    assert codec.parse_topic("lumenctl/", "lumenctl/desktop/status").kind is codec.TopicKind.STATUS
    hub = codec.parse_topic("lumenctl", "lumenctl/desktop/info")
    assert hub.kind is codec.TopicKind.HUB_INFO
    assert hub.node_id == "desktop"
    assert hub.monitor_key is None
    monitor = codec.parse_topic("lumenctl", "lumenctl/desktop/mate/info")
    assert monitor.kind is codec.TopicKind.MONITOR_INFO
    assert monitor.monitor_key == "mate"


def test_parse_topic_ignores_state_commands_and_other_prefixes():
    assert codec.parse_topic("lumenctl", "lumenctl/desktop/mate/state") is None
    assert codec.parse_topic("lumenctl", "lumenctl/desktop/mate/light/set") is None
    assert codec.parse_topic("lumenctl", "lumenctl/desktop/command") is None
    assert codec.parse_topic("lumenctl", "lumenctl2/desktop/status") is None
    assert codec.parse_topic("lumenctl", "lumenctl/desktop//info") is None


def test_node_filter():
    assert codec.parse_topic("lumenctl", "lumenctl/laptop/status", "desktop") is None
    kept = codec.parse_topic("lumenctl", "lumenctl/desktop/status", " desktop ")
    assert kept is not None
    assert kept.node_id == "desktop"


def test_subscription_topics():
    assert codec.subscription_topics("lumenctl/") == (
        "lumenctl/+/status",
        "lumenctl/+/info",
        "lumenctl/+/+/info",
    )


def test_decode_monitor_uses_the_topic_key_and_builds_entities():
    info = codec.decode_monitor_payload(MONITOR, "mate")
    assert info is not None
    assert info.key == "mate"
    assert info.manufacturer == "HWV"
    assert info.serial_number == "ABC"
    assert info.power == "on"
    assert info.brightness is not None
    assert (info.brightness.current, info.brightness.min, info.brightness.max) == (40, 0, 100)
    assert info.volume is None
    assert info.inputs == ["HDMI1", "DisplayPort1"]
    assert info.display_modes == []
    assert codec.entity_kinds(info) == {"light", "contrast", "ddc", "input", "color_preset"}


def test_missing_brightness_and_empty_lists_drop_entities():
    payload = json.dumps({"name": "Panel", "brightness": None, "inputs": [], "ddc_ok": False})
    info = codec.decode_monitor_payload(payload, "panel")
    assert info is not None
    assert info.power == "on"
    assert codec.entity_kinds(info) == {"ddc"}
    assert codec.enabled_by_default("red") is False
    assert codec.enabled_by_default("contrast") is True


def test_empty_payload_removes_the_monitor_and_garbage_raises():
    assert codec.decode_monitor_payload("  \n", "mate") is None
    with pytest.raises(json.JSONDecodeError):
        codec.decode_monitor_payload("{", "mate")
    with pytest.raises(ValueError):
        codec.decode_monitor_payload("null", "mate")
    with pytest.raises(ValueError):
        codec.decode_monitor_payload(json.dumps({"brightness": True}), "mate")
    with pytest.raises(ValueError):
        codec.decode_monitor_payload(json.dumps({"power": "sleep"}), "mate")


def test_range_widens_to_the_current_value():
    info = codec.decode_monitor_payload(
        json.dumps({"brightness": {"current": 5, "min": 10, "max": 80}, "power": "standby"}),
        "mate",
    )
    assert info is not None
    assert info.power == "standby"
    assert info.brightness is not None
    assert (info.brightness.current, info.brightness.min, info.brightness.max) == (5, 5, 80)


def test_brightness_round_trip():
    value = codec.RangeValue(current=40, min=0, max=100)
    assert codec.range_to_ha_brightness(value) == 102
    assert codec.ha_brightness_to_native(value, 102) == 40
    assert codec.ha_brightness_to_native(value, 255) == 100
    limited = codec.RangeValue(current=10, min=10, max=100)
    assert codec.range_to_ha_brightness(limited) == 0
    assert codec.ha_brightness_to_native(limited, 0) == 10
    assert codec.ha_brightness_to_native(limited, 255) == 100
    flat = codec.RangeValue(current=4, min=4, max=4)
    assert codec.range_to_ha_brightness(flat) == 255
    assert codec.ha_brightness_to_native(flat, 0) == 4


def test_light_command_and_topics():
    assert codec.light_command("ON") == '{"state":"ON"}'
    assert codec.light_command("ON", 40) == '{"state":"ON","brightness":40}'
    assert codec.light_command("OFF") == '{"state":"OFF"}'
    assert codec.monitor_set_topic("lumenctl/", "desktop", "mate", "contrast") == "lumenctl/desktop/mate/contrast/set"
    assert codec.hub_topic("lumenctl", "desktop", "command") == "lumenctl/desktop/command"


def test_identifiers_and_platforms():
    assert codec.hub_identifier("desktop") == "lumenctl_desktop"
    assert codec.monitor_identifier("desktop", "mate") == "lumenctl_desktop_mate"
    assert codec.entity_unique_id("desktop", "light", "mate") == "lumenctl_desktop_mate_light"
    assert codec.entity_unique_id("desktop", "command") == "lumenctl_desktop_command"
    assert codec.platform_for("volume") == "number"
    assert codec.platform_for("input") == "select"
    assert codec.platform_for("ddc") == "binary_sensor"
    assert codec.platform_for("command") == "text"
    assert codec.platform_for("rescan") == "button"
    with pytest.raises(KeyError):
        codec.platform_for("nope")


def test_hub_info_and_settings():
    info = codec.parse_hub_info('{"node_id":"other","name":" Office ","sw_version":"1.0.0"}', "desktop")
    assert info.node_id == "desktop"
    assert info.name == "Office"
    assert info.sw_version == "1.0.0"
    fallback = codec.parse_hub_info("{}", "desktop")
    assert fallback.name == "lumenctl (desktop)"
    assert fallback.sw_version is None
    base, node = codec.merge_settings({"base_topic": "lumenctl", "node_id": ""}, {"node_id": "desktop"})
    assert base == "lumenctl"
    assert node == "desktop"
    base, node = codec.merge_settings({}, None)
    assert (base, node) == ("lumenctl", None)


def test_checked_topics_reject_wildcards():
    assert codec.checked_base_topic(" lumenctl/ ") == "lumenctl"
    assert codec.checked_node_filter("  ") is None
    with pytest.raises(ValueError, match="invalid_base_topic"):
        codec.checked_base_topic("home/+/pc")
    with pytest.raises(ValueError, match="invalid_base_topic"):
        codec.checked_base_topic("   ")
    with pytest.raises(ValueError, match="invalid_node_id"):
        codec.checked_node_filter("desk top")
    with pytest.raises(ValueError, match="invalid_node_id"):
        codec.checked_node_filter("a/b")
