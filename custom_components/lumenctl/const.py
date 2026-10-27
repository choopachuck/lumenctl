"""Constants for the lumenctl integration."""

DOMAIN = "lumenctl"

CONF_BASE_TOPIC = "base_topic"
CONF_NODE_ID = "node_id"
DEFAULT_BASE_TOPIC = "lumenctl"

ENTITY_NAMES = {
    "contrast": "Contrast",
    "volume": "Speaker volume",
    "red": "Red",
    "green": "Green",
    "blue": "Blue",
    "input": "Input",
    "color_preset": "Color preset",
    "display_mode": "Display mode",
    "ddc": "DDC/CI",
    "command": "Command",
    "rescan": "Rescan monitors",
}

ICONS = {
    "light": "mdi:monitor",
    "contrast": "mdi:contrast-box",
    "volume": "mdi:volume-high",
    "red": "mdi:palette",
    "green": "mdi:palette",
    "blue": "mdi:palette",
    "input": "mdi:video-input-hdmi",
    "color_preset": "mdi:palette-swatch",
    "display_mode": "mdi:movie-open",
    "ddc": "mdi:lan-connect",
    "command": "mdi:console-line",
    "rescan": "mdi:monitor-shimmer",
}
