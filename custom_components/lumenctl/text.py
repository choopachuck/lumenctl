"""Raw lumenctl command line."""

from __future__ import annotations

from homeassistant.components.text import TextEntity, TextMode
from homeassistant.config_entries import ConfigEntry
from homeassistant.const import Platform
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddEntitiesCallback

from .codec import hub_topic
from .const import DOMAIN
from .entity import LumenNodeEntity
from .hub import LumenHub


async def async_setup_entry(
    hass: HomeAssistant,
    entry: ConfigEntry,
    async_add_entities: AddEntitiesCallback,
) -> None:
    hub: LumenHub = hass.data[DOMAIN][entry.entry_id]
    hub.register_node_platform(Platform.TEXT, async_add_entities, LumenCommand)


class LumenCommand(LumenNodeEntity, TextEntity):
    """The state is the last command sent during this Home Assistant session."""

    _attr_mode = TextMode.TEXT
    _attr_native_min = 0
    _attr_native_max = 1024

    def __init__(self, hub: LumenHub, node_id: str) -> None:
        super().__init__(hub, node_id, "command")
        self._value = ""

    @property
    def native_value(self) -> str:
        return self._value

    async def async_set_value(self, value: str) -> None:
        await self.hub.async_publish(hub_topic(self.hub.base_topic, self.node_id, "command"), value)
        self._value = value
        self.schedule_state()
