"""Monitor backlight and power."""

from __future__ import annotations

from typing import Any

from homeassistant.components.light import ATTR_BRIGHTNESS, ColorMode, LightEntity
from homeassistant.config_entries import ConfigEntry
from homeassistant.const import Platform
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddEntitiesCallback

from .codec import ha_brightness_to_native, light_command, monitor_set_topic, range_to_ha_brightness
from .const import DOMAIN
from .entity import LumenMonitorEntity
from .hub import LumenHub


async def async_setup_entry(
    hass: HomeAssistant,
    entry: ConfigEntry,
    async_add_entities: AddEntitiesCallback,
) -> None:
    hub: LumenHub = hass.data[DOMAIN][entry.entry_id]
    hub.register_monitor_platform(Platform.LIGHT, async_add_entities, LumenLight)


class LumenLight(LumenMonitorEntity, LightEntity):
    """Brightness slider. The entity name is the monitor name."""

    _attr_color_mode = ColorMode.BRIGHTNESS
    _attr_supported_color_modes = {ColorMode.BRIGHTNESS}

    def __init__(self, hub: LumenHub, node_id: str, key: str, kind: str) -> None:
        super().__init__(hub, node_id, key, kind, named=False)

    @property
    def is_on(self) -> bool:
        info = self._info()
        return info is not None and info.power == "on"

    @property
    def brightness(self) -> int | None:
        info = self._info()
        if info is None or info.brightness is None:
            return None
        return range_to_ha_brightness(info.brightness)

    async def async_turn_on(self, **kwargs: Any) -> None:
        info = self._info()
        if info is None or info.brightness is None:
            return
        native = None
        if ATTR_BRIGHTNESS in kwargs:
            native = ha_brightness_to_native(info.brightness, int(kwargs[ATTR_BRIGHTNESS]))
        await self.hub.async_publish(
            monitor_set_topic(self.hub.base_topic, self.node_id, self.key, "light"),
            light_command("ON", native),
        )
        info.power = "on"
        if native is not None:
            info.brightness.current = native
        self.schedule_state()

    async def async_turn_off(self, **kwargs: Any) -> None:
        info = self._info()
        await self.hub.async_publish(
            monitor_set_topic(self.hub.base_topic, self.node_id, self.key, "light"),
            light_command("OFF"),
        )
        if info is not None:
            info.power = "off"
            self.schedule_state()
