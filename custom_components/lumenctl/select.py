"""Input, color preset and display mode."""

from __future__ import annotations

from homeassistant.components.select import SelectEntity
from homeassistant.config_entries import ConfigEntry
from homeassistant.const import Platform
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddEntitiesCallback

from .codec import SELECT_CURRENT, SELECT_KINDS, monitor_set_topic
from .const import DOMAIN
from .entity import LumenMonitorEntity
from .hub import LumenHub


async def async_setup_entry(
    hass: HomeAssistant,
    entry: ConfigEntry,
    async_add_entities: AddEntitiesCallback,
) -> None:
    hub: LumenHub = hass.data[DOMAIN][entry.entry_id]
    hub.register_monitor_platform(Platform.SELECT, async_add_entities, LumenSelect)


class LumenSelect(LumenMonitorEntity, SelectEntity):
    """A named VCP choice. Options come from the monitor info payload."""

    def __init__(self, hub: LumenHub, node_id: str, key: str, kind: str) -> None:
        super().__init__(hub, node_id, key, kind)

    @property
    def options(self) -> list[str]:
        info = self._info()
        if info is None:
            return []
        values = getattr(info, SELECT_KINDS[self.kind])
        return list(values)

    @property
    def current_option(self) -> str | None:
        info = self._info()
        if info is None:
            return None
        current = getattr(info, SELECT_CURRENT[self.kind])
        if current in self.options:
            return current
        return None

    async def async_select_option(self, option: str) -> None:
        await self.hub.async_publish(
            monitor_set_topic(self.hub.base_topic, self.node_id, self.key, self.kind),
            option,
        )
        info = self._info()
        if info is not None:
            setattr(info, SELECT_CURRENT[self.kind], option)
        self.schedule_state()
