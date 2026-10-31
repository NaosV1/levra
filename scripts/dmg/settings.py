# Levra's disk image, as dmgbuild builds it. scripts/make-dmg.sh passes the
# paths with -D; the window size and the icons' centres come from the
# layout.json that scripts/artwork.swift writes next to the background.
# https://dmgbuild.readthedocs.io/en/stable/settings.html
import json
import os.path

application = defines["app"]
with open(defines["layout"]) as file:
    layout = json.load(file)

format = "UDZO"
files = [application]
symlinks = {"Applications": "/Applications"}
icon = defines["icon"]
background = defines["background"]

# The Finder cannot centre a window: this puts it near the top left.
window_rect = ((200, 100000), tuple(layout["window"]))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
show_icon_preview = False

icon_size = 128
text_size = 13
icon_locations = {
    os.path.basename(application): tuple(layout["app"]),
    "Applications": tuple(layout["applications"]),
}
