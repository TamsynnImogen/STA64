#!/usr/bin/env python3
"""Regenerate this fixture's synthetic BinaryMode v3021 menu.sol."""

from pathlib import Path
import math
import struct


OUT = Path(__file__).parent / "bzn" / "menu.sol"


class Sol:
    def __init__(self):
        self.data = bytearray(b"version [1] =\r\n3021\r\nBinaryMode [1] =\r\ntrue\r\n")
        self.next_id = 1

    def rec(self, tag, payload):
        self.data.extend(struct.pack("<II", tag, len(payload)))
        self.data.extend(payload)

    def u32(self, value):
        self.rec(4, struct.pack("<I", value))

    def vec3(self, value):
        self.rec(9, struct.pack("<3f", *value))

    def pair(self, value):
        raw = value.encode("ascii") + b"\0"
        self.u32(len(raw))
        self.rec(2, raw)

    def filler(self, count):
        for _ in range(count):
            self.rec(1, b"\1")

    def object(self, odf, instance, pos, yaw=0.0):
        odf_raw = odf.encode("ascii") + b"\0"
        name = instance.encode("ascii")
        if len(name) > 39:
            raise ValueError(f"instance name too long: {instance}")
        self.rec(3, struct.pack("<H", len(odf_raw)))
        self.rec(2, odf_raw)
        object_id = self.next_id
        self.next_id += 1
        self.u32(object_id)
        self.vec3(pos)
        self.u32(0)  # neutral team; this showcase is not a playable map
        self.rec(2, name.ljust(40, b"\0"))
        self.rec(8, struct.pack("<I", object_id))
        c, s = math.cos(yaw), math.sin(yaw)
        # Matrix34: right, up, front, position.
        self.rec(11, struct.pack("<12f", c, 0, -s, 0, 1, 0, s, 0, c, *pos))

    def write(self):
        self.filler(4)
        for value in ("mp", "{menu-showcase}", "Original Menu Showcase", "bzn/menu.sol", "menu.sol"):
            self.pair(value)
        self.filler(1)
        self.vec3((-1200, -600, -1400))
        self.vec3((2400, 1200, 2800))
        self.filler(2)
        self.u32(0)
        self.rec(2, b"")
        self.filler(6)
        self.u32(0)  # no player starts
        placements = [
            ("fscout", "escort", (-150, 0, -40), -0.25),
            ("fgalaxy", "flagship", (80, 0, 35), 0.85),
            ("fbase", "bastion", (5, 0, 205), math.pi),
        ]
        marker_rows = [
            # Name, position, yaw. These named, synthetic waypoints are the
            # route controls referenced by missions/menu.lua.
            ("menu_camera_main", (-300, 190, -340), 0.55),
            ("menu_camera_main_arc", (-170, 150, -430), 0.20),
            ("menu_camera_main_return", (-420, 180, -410), 0.85),
            ("menu_target_main", (-5, 20, 55), 0),
            ("menu_target_main_arc", (25, 15, 50), 0),
            ("menu_target_main_return", (-20, 20, 65), 0),
            ("menu_camera_fleet", (-350, 95, -105), 0.60),
            ("menu_camera_fleet_arc", (-230, 80, -180), 0.25),
            ("menu_camera_fleet_return", (-390, 105, -210), 0.90),
            ("menu_target_fleet", (-140, 5, -25), 0),
            ("menu_target_fleet_arc", (-115, 5, -5), 0),
            ("menu_target_fleet_return", (-160, 5, -35), 0),
            ("menu_camera_planning", (300, 175, -175), -0.85),
            ("menu_camera_planning_arc", (390, 145, -70), -1.05),
            ("menu_camera_planning_return", (245, 155, -70), -0.65),
            ("menu_target_planning", (75, 15, 40), 0),
            ("menu_target_planning_arc", (90, 12, 30), 0),
            ("menu_target_planning_return", (60, 15, 50), 0),
            ("menu_camera_dock", (20, 100, 495), math.pi),
            ("menu_camera_dock_arc", (125, 75, 450), 2.75),
            ("menu_camera_dock_return", (-95, 85, 455), -2.75),
            ("menu_target_dock", (5, 10, 205), 0),
            ("menu_target_dock_arc", (20, 10, 205), 0),
            ("menu_target_dock_return", (-10, 10, 200), 0),
            ("menu_camera_settings", (360, 150, 340), -2.45),
            ("menu_camera_settings_arc", (285, 125, 430), -2.20),
            ("menu_camera_settings_return", (430, 140, 420), -2.75),
            ("menu_target_settings", (65, 5, 115), 0),
            ("menu_target_settings_arc", (80, 5, 130), 0),
            ("menu_target_settings_return", (55, 5, 100), 0),
            ("menu_camera_multiplayer", (-420, 205, 300), 2.30),
            ("menu_camera_multiplayer_arc", (-315, 165, 375), 2.55),
            ("menu_camera_multiplayer_return", (-485, 180, 410), 2.10),
            ("menu_target_multiplayer", (-20, 20, 75), 0),
            ("menu_target_multiplayer_arc", (0, 20, 85), 0),
            ("menu_target_multiplayer_return", (-40, 15, 70), 0),
            ("menu_camera_campaign", (245, 215, 545), -2.85),
            ("menu_camera_campaign_arc", (95, 175, 610), -2.55),
            ("menu_camera_campaign_return", (350, 190, 590), -3.00),
            ("menu_target_campaign", (0, 10, 200), 0),
            ("menu_target_campaign_arc", (-15, 10, 190), 0),
            ("menu_target_campaign_return", (20, 10, 210), 0),
            ("menu_point_escort_wide", (-80, 18, -125), 1.1),
            ("menu_point_escort_close", (-210, 24, 45), -1.2),
            ("menu_point_flagship_wide", (145, 20, 130), 2.4),
            ("menu_point_flagship_close", (25, 22, -45), -0.6),
        ]
        self.u32(len(placements) + len(marker_rows))
        for row in placements:
            self.object(*row)
        for name, pos, yaw in marker_rows:
            self.object("fscout", name, pos, yaw)
        OUT.parent.mkdir(parents=True, exist_ok=True)
        OUT.write_bytes(self.data)
        print(f"wrote {OUT} ({len(self.data)} bytes, {len(placements) + len(marker_rows)} placements)")


if __name__ == "__main__":
    Sol().write()
