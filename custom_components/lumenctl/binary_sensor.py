"""DDC/CI link status."""

from __future__ import annotations

from homeassistant.components.binary_sensor import BinarySensorDeviceClass, BinarySensorEntity
from homeassistant.config_entries import ConfigEntry
from homeassistant.const import Platform
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity import EntityCategory
from homeassistant.helpers.entity_platform import AddEntitiesCallback

from .const import DOMAIN
from .entity import LumenMonitorEntity
from .hub import LumenHub


async def async_setup_entry(
    hass: HomeAssistant,
    entry: ConfigEntry,
    async_add_entities: AddEntitiesCallback,
) -> None:
    hub: LumenHub = hass.data[DOMAIN][entry.entry_id]
    hub.register_monitor_platform(Platform.BINARY_SENSOR, async_add_entities, LumenDdc)


class LumenDdc(LumenMonitorEntity, BinarySensorEntity):
    """True while the monitor answers DDC/CI."""

    _attr_device_class = BinarySensorDeviceClass.CONNECTIVITY
    _attr_entity_category = EntityCategory.DIAGNOSTIC

    def __init__(self, hub: LumenHub, node_id: str, key: str, kind: str) -> None:
        super().__init__(hub, node_id, key, kind)

    @property
    def is_on(self) -> bool | None:
        info = self._info()
        if info is None:
            return None
        return info.ddc_ok
