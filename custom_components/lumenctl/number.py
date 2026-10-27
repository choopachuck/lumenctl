"""Contrast, volume and RGB gain sliders."""

from __future__ import annotations

from homeassistant.components.number import NumberEntity, NumberMode
from homeassistant.config_entries import ConfigEntry
from homeassistant.const import Platform
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddEntitiesCallback

from .codec import RangeValue, monitor_set_topic
from .const import DOMAIN
from .entity import LumenMonitorEntity
from .hub import LumenHub


async def async_setup_entry(
    hass: HomeAssistant,
    entry: ConfigEntry,
    async_add_entities: AddEntitiesCallback,
) -> None:
    hub: LumenHub = hass.data[DOMAIN][entry.entry_id]
    hub.register_monitor_platform(Platform.NUMBER, async_add_entities, LumenNumber)


class LumenNumber(LumenMonitorEntity, NumberEntity):
    """One continuous VCP value."""

    _attr_mode = NumberMode.SLIDER
    _attr_native_step = 1

    def __init__(self, hub: LumenHub, node_id: str, key: str, kind: str) -> None:
        super().__init__(hub, node_id, key, kind)

    def _range(self) -> RangeValue | None:
        info = self._info()
        if info is None:
            return None
        value = getattr(info, self.kind)
        return value if isinstance(value, RangeValue) else None

    @property
    def native_min_value(self) -> float:
        value = self._range()
        return 0 if value is None else value.min

    @property
    def native_max_value(self) -> float:
        value = self._range()
        return 100 if value is None else value.max

    @property
    def native_value(self) -> float | None:
        value = self._range()
        return None if value is None else value.current

    async def async_set_native_value(self, value: float) -> None:
        raw = int(round(value))
        await self.hub.async_publish(
            monitor_set_topic(self.hub.base_topic, self.node_id, self.key, self.kind),
            str(raw),
        )
        current = self._range()
        if current is not None:
            current.current = raw
        self.schedule_state()
