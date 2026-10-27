"""Rescan the monitors attached to a computer."""

from __future__ import annotations

from homeassistant.components.button import ButtonEntity
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
    hub.register_node_platform(Platform.BUTTON, async_add_entities, LumenRescan)


class LumenRescan(LumenNodeEntity, ButtonEntity):
    """Ask lumenctl to look for monitors again."""

    def __init__(self, hub: LumenHub, node_id: str) -> None:
        super().__init__(hub, node_id, "rescan")

    async def async_press(self) -> None:
        await self.hub.async_publish(hub_topic(self.hub.base_topic, self.node_id, "rescan"), "PRESS")
