"""Thoric's wallpaper, the default of both desktop sessions. Each session finds it through files of
its own, the xfconf channel XFCE reads and the schema override and wallpaper list GNOME reads, and
those name the path the Dockerfile copies the picture to and the gray around it. A path that
drifts leaves a desktop without its mark, a gray that drifts gives the sessions, or GNOME's
default and its Settings, different colors, and an override the schemas are not compiled with
again changes nothing."""

from configparser import ConfigParser
import re
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[3]
DESKTOP = ROOT / "images/desktop"
WALLPAPER = DESKTOP / "wallpaper"
STAGED = "/usr/local/share/thoric/wallpaper/"


def stages():
    """Each stage of the Dockerfile as its lines."""
    lines, stage = {}, None
    for line in (DESKTOP / "Dockerfile").read_text().splitlines():
        if match := re.match(r"FROM \S+ AS (\S+)$", line):
            stage = match.group(1)
        lines.setdefault(stage, []).append(line)
    return lines


def copies(stage):
    """Where a stage puts each file of the staged wallpaper folder, by the file's name."""
    placed = {}
    for line in stages()[stage]:
        if match := re.match(rf"COPY --from=scripts {re.escape(STAGED)}(\S+) (\S+)$", line):
            name, target = match.groups()
            placed[name] = target + name if target.endswith("/") else target
    return placed


def override():
    keys = ConfigParser(interpolation=None)
    keys.read(WALLPAPER / "90_thoric.gschema.override")
    return {key: value.strip("'") for key, value in keys["org.gnome.desktop.background"].items()}


GRAY = override()["primary-color"]


class WallpaperTest(unittest.TestCase):
    def test_the_scripts_stage_holds_every_file_a_session_copies(self):
        self.assertIn(f"COPY wallpaper/ {STAGED}", stages()["scripts"])
        used = set(copies("xfce")) | set(copies("gnome"))
        self.assertEqual(used, {path.name for path in WALLPAPER.iterdir()})

    def test_xfce_draws_it_on_every_workspace_on_gray(self):
        placed = copies("xfce")
        channel = ET.parse(WALLPAPER / "xfce4-desktop.xml").getroot()
        self.assertEqual(placed["xfce4-desktop.xml"], f"/etc/xdg/xfce4/xfconf/xfce-perchannel-xml/{channel.get('name')}.xml")
        workspaces = channel.findall(".//property[@name='monitorVNC-0']/property")
        self.assertEqual([w.get("name") for w in workspaces], [f"workspace{i}" for i in range(4)])
        for workspace in workspaces:
            values = {p.get("name"): p for p in workspace}
            self.assertEqual(values["last-image"].get("value"), placed["thoric.svg"])
            rgba = [round(float(v.get("value")) * 255) for v in values["rgba1"]]
            self.assertEqual("#{:02x}{:02x}{:02x}".format(*rgba[:3]), GRAY)
            self.assertEqual(rgba[3], 255)

    def test_gnome_defaults_to_it_on_gray_and_lists_it(self):
        placed = copies("gnome")
        keys = override()
        self.assertEqual(keys["picture-uri"], f"file://{placed['thoric.svg']}")
        self.assertEqual(keys["picture-uri-dark"], keys["picture-uri"])
        listed = ET.parse(WALLPAPER / "gnome-wallpapers.xml").getroot().find("wallpaper")
        self.assertEqual(listed.findtext("filename"), placed["thoric.svg"])
        self.assertEqual(listed.findtext("options"), keys["picture-options"])
        self.assertEqual({listed.findtext("pcolor"), listed.findtext("scolor"), keys["secondary-color"]}, {GRAY})
        self.assertTrue(placed["gnome-wallpapers.xml"].startswith("/usr/share/gnome-background-properties/"))
        # glib reads overrides only when the schemas are compiled, after the override is in place.
        lines = stages()["gnome"]
        copied = next(i for i, line in enumerate(lines) if "90_thoric.gschema.override" in line)
        self.assertIn("RUN glib-compile-schemas /usr/share/glib-2.0/schemas", lines[copied:])


if __name__ == "__main__":
    unittest.main()
