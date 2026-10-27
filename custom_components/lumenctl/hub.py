"""MQTT subscription and the devices discovered from it."""

from __future__ import annotations

import logging
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any

from homeassistant.components import mqtt
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant
from homeassistant.helpers import device_registry as dr

from .codec import (
    HubInfo,
    MonitorInfo,
    TopicKind,
    decode_monitor_payload,
    entity_kinds,
    hub_identifier,
    merge_settings,
    monitor_identifier,
    parse_hub_info,
    parse_topic,
    platform_for,
    subscription_topics,
)
from .const import CONF_BASE_TOPIC, CONF_NODE_ID, DEFAULT_BASE_TOPIC, DOMAIN
from .entity import LumenBaseEntity

_LOGGER = logging.getLogger(__name__)

AddEntities = Callable[[list[Any]], None]
MonitorFactory = Callable[["LumenHub", str, str, str], LumenBaseEntity]
NodeFactory = Callable[["LumenHub", str], LumenBaseEntity]


@dataclass
class NodeRuntime:
    """One lumenctl process and the monitors it last described."""

    node_id: str
    online: bool = False
    name: str = ""
    sw_version: str | None = None
    monitors: dict[str, MonitorInfo] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if not self.name:
            self.name = f"lumenctl ({self.node_id})"


class LumenHub:
    """Tracks computers and monitors advertised under one base topic."""

    def __init__(self, hass: HomeAssistant, entry: ConfigEntry) -> None:
        self.hass = hass
        self.entry = entry
        self.base_topic, self.node_filter = merge_settings(
            dict(entry.data),
            dict(entry.options),
            base_key=CONF_BASE_TOPIC,
            node_key=CONF_NODE_ID,
            default_base=DEFAULT_BASE_TOPIC,
        )
        self.nodes: dict[str, NodeRuntime] = {}
        self._entities: dict[tuple[str, str, str, str], LumenBaseEntity] = {}
        self._monitor_factories: dict[str, tuple[AddEntities, MonitorFactory]] = {}
        self._node_factories: dict[str, tuple[AddEntities, NodeFactory]] = {}
        self._unsubs: list[Callable[[], None]] = []

    def register_monitor_platform(self, platform: str, async_add: AddEntities, factory: MonitorFactory) -> None:
        self._monitor_factories[str(platform)] = (async_add, factory)

    def register_node_platform(self, platform: str, async_add: AddEntities, factory: NodeFactory) -> None:
        self._node_factories[str(platform)] = (async_add, factory)

    async def async_start(self) -> None:
        self.unsubscribe()
        for topic in subscription_topics(self.base_topic):
            self._unsubs.append(
                await mqtt.async_subscribe(self.hass, topic, self._on_message, qos=1, encoding="utf-8")
            )

    def unsubscribe(self) -> None:
        for unsub in self._unsubs:
            unsub()
        self._unsubs.clear()

    def is_online(self, node_id: str) -> bool:
        node = self.nodes.get(node_id)
        return node is not None and node.online

    def node_state(self, node_id: str) -> NodeRuntime | None:
        return self.nodes.get(node_id)

    def node_name(self, node_id: str) -> str:
        node = self.nodes.get(node_id)
        if node is None or not node.name:
            return f"lumenctl ({node_id})"
        return node.name

    def monitor_info(self, node_id: str, key: str) -> MonitorInfo | None:
        node = self.nodes.get(node_id)
        if node is None:
            return None
        return node.monitors.get(key)

    async def async_publish(self, topic: str, payload: str) -> None:
        await mqtt.async_publish(self.hass, topic, payload, qos=1, retain=False)

    async def _on_message(self, msg: mqtt.ReceiveMessage) -> None:
        try:
            parsed = parse_topic(self.base_topic, msg.topic, self.node_filter)
            if parsed is None:
                return
            payload = msg.payload if isinstance(msg.payload, str) else bytes(msg.payload).decode("utf-8", "replace")
            if parsed.kind is TopicKind.STATUS:
                await self._set_online(parsed.node_id, payload.strip() == "online")
            elif parsed.kind is TopicKind.HUB_INFO:
                if not payload.strip():
                    return
                await self._apply_hub_info(parsed.node_id, parse_hub_info(payload, parsed.node_id))
            elif parsed.kind is TopicKind.MONITOR_INFO:
                key = parsed.monitor_key or ""
                info = decode_monitor_payload(payload, key)
                if info is None:
                    await self._remove_monitor(parsed.node_id, key)
                else:
                    await self._upsert_monitor(parsed.node_id, info)
        except Exception:
            _LOGGER.exception("Failed to handle MQTT message on %s", msg.topic)

    async def _set_online(self, node_id: str, online: bool) -> None:
        node = self._ensure_node(node_id)
        node.online = online
        self._sync_hub_device(node_id)
        self._ensure_hub_entities(node_id)
        self._touch_node(node_id)

    async def _apply_hub_info(self, node_id: str, info: HubInfo) -> None:
        node = self._ensure_node(node_id)
        node.name = info.name
        node.sw_version = info.sw_version
        self._sync_hub_device(node_id)
        self._ensure_hub_entities(node_id)
        self._touch_node(node_id)

    async def _upsert_monitor(self, node_id: str, info: MonitorInfo) -> None:
        self._ensure_node(node_id)
        self._sync_hub_device(node_id)
        self._ensure_hub_entities(node_id)
        self.nodes[node_id].monitors[info.key] = info
        self._sync_monitor_device(node_id, info)
        desired = entity_kinds(info)
        existing = {
            key[3]
            for key in self._entities
            if key[0] == "monitor" and key[1] == node_id and key[2] == info.key
        }
        for kind in sorted(desired - existing):
            self._add_monitor_entity(node_id, info.key, kind)
        for kind in existing - desired:
            await self._remove_kind(node_id, info.key, kind)
        for kind in desired & existing:
            self._entities[("monitor", node_id, info.key, kind)].schedule_state()

    async def _remove_monitor(self, node_id: str, key: str) -> None:
        node = self.nodes.get(node_id)
        if node is not None:
            node.monitors.pop(key, None)
        doomed = [entity_key for entity_key in self._entities if entity_key[:3] == ("monitor", node_id, key)]
        for entity_key in doomed:
            await self._entities.pop(entity_key).mark_removed()
        registry = dr.async_get(self.hass)
        device = registry.async_get_device(identifiers={(DOMAIN, monitor_identifier(node_id, key))})
        if device is not None:
            registry.async_remove_device(device.id)
        _LOGGER.debug("Removed monitor %s/%s", node_id, key)

    def _ensure_node(self, node_id: str) -> NodeRuntime:
        node = self.nodes.get(node_id)
        if node is None:
            node = NodeRuntime(node_id=node_id)
            self.nodes[node_id] = node
        return node

    def _ensure_hub_entities(self, node_id: str) -> None:
        for kind in ("command", "rescan"):
            entity_key = ("node", node_id, "", kind)
            if entity_key in self._entities:
                continue
            found = self._node_factories.get(platform_for(kind))
            if found is None:
                _LOGGER.error("Platform for %s is not registered", kind)
                continue
            async_add, factory = found
            entity = factory(self, node_id)
            self._entities[entity_key] = entity
            async_add([entity])

    def _add_monitor_entity(self, node_id: str, key: str, kind: str) -> None:
        found = self._monitor_factories.get(platform_for(kind))
        if found is None:
            _LOGGER.error("Platform for %s is not registered", kind)
            return
        async_add, factory = found
        entity = factory(self, node_id, key, kind)
        self._entities[("monitor", node_id, key, kind)] = entity
        async_add([entity])

    async def _remove_kind(self, node_id: str, key: str, kind: str) -> None:
        entity = self._entities.pop(("monitor", node_id, key, kind), None)
        if entity is not None:
            await entity.mark_removed()

    def _touch_node(self, node_id: str) -> None:
        for key, entity in self._entities.items():
            if key[1] == node_id:
                entity.schedule_state()

    def _sync_hub_device(self, node_id: str) -> None:
        node = self.nodes[node_id]
        dr.async_get(self.hass).async_get_or_create(
            config_entry_id=self.entry.entry_id,
            identifiers={(DOMAIN, hub_identifier(node_id))},
            name=node.name,
            manufacturer="lumenctl",
            model="lumenctl",
            sw_version=node.sw_version,
        )

    def _sync_monitor_device(self, node_id: str, info: MonitorInfo) -> None:
        dr.async_get(self.hass).async_get_or_create(
            config_entry_id=self.entry.entry_id,
            identifiers={(DOMAIN, monitor_identifier(node_id, info.key))},
            name=info.name,
            manufacturer=info.manufacturer,
            model=info.model,
            serial_number=info.serial_number,
            via_device=(DOMAIN, hub_identifier(node_id)),
        )
