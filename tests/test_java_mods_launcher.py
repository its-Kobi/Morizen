import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools import java_mods_launcher as kit


def write(path: Path, text: str = "") -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def version(home: Path, name: str, *, complete=True, client=True, resources=True) -> Path:
    folder = home / "versions" / name
    folder.mkdir(parents=True)
    if complete:
        write(folder / kit.COMPLETE)
    if client:
        write(folder / kit._exe("horizon-client"), "game")
    if resources:
        write(
            folder
            / ".local"
            / "horizon-client"
            / "26.3"
            / kit.RESOURCES_MANIFEST,
            "{}",
        )
    return folder


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.home = self.root / "launcher"
        self.kit = self.root / "kit"
        for name in kit.BINARIES:
            write(self.kit / kit._exe(name), name)

    def tearDown(self):
        self.temp.cleanup()

    def test_picks_the_newest_prepared_version(self):
        old = version(self.home, "a")
        new = version(self.home, "b")
        version(self.home, "unprepared", resources=False)
        version(self.home, "incomplete", complete=False)
        os.utime(old / kit.COMPLETE, (1, 1))
        os.utime(new / kit.COMPLETE, (2, 2))
        self.assertEqual(kit.pick_source(self.home), new)

    def test_no_prepared_version_is_reported(self):
        version(self.home, "a", resources=False)
        self.assertIsNone(kit.pick_source(self.home))

    def test_config_falls_back_through_checkout_then_kit(self):
        self.assertIsNone(kit.bridge_config(self.kit, None, None))
        built = self.kit / ".local" / "java-mod-bridge" / "in-game" / kit.CONFIG
        write(built, "{}")
        self.assertEqual(kit.bridge_config(self.kit, None, None), built.resolve())
        write(self.kit / kit.FOLDER / kit.CONFIG, "{}")
        checkout = self.root / "checkout"
        known = checkout / ".local" / "java-mod-bridge" / "in-game" / kit.CONFIG
        write(known, "{}")
        self.assertEqual(kit.bridge_config(self.kit, checkout, None), known.resolve())

    def test_settings_keep_every_other_key(self):
        write(self.home / "settings.json", json.dumps({"appearance": "Light", "selected": "x"}))
        config = self.root / "config.json"
        write(config, "{}")
        path = kit.save_settings(self.home, "test", config)
        settings = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual(settings["appearance"], "Light")
        self.assertEqual(settings["selected"], "test")
        self.assertEqual(settings["java_mods_config"], str(config))

    def test_install_copies_the_prepared_version_and_swaps_the_game(self):
        source = version(self.home, "old")
        game = self.root / "game"
        config = self.root / "config.json"
        write(config, "{}")
        with mock.patch.object(kit, "game_home", return_value=game):
            target = kit.install(self.kit, self.home, "test", config)
        self.assertTrue((target / kit.COMPLETE).is_file())
        self.assertTrue(
            (target / ".local" / "horizon-client" / "26.3" / kit.RESOURCES_MANIFEST).is_file()
        )
        self.assertEqual((target / kit._exe("horizon-client")).read_text(), "horizon-client")
        self.assertEqual((source / kit._exe("horizon-client")).read_text(), "game")
        self.assertTrue((game / kit.FOLDER / kit.CONFIG).is_file())

    def test_missing_config_does_not_touch_the_launcher(self):
        version(self.home, "old")
        with mock.patch.object(kit, "game_home", return_value=self.root / "game"):
            with self.assertRaises(SystemExit):
                kit.install(self.kit, self.home)
        self.assertFalse((self.home / "settings.json").exists())


if __name__ == "__main__":
    unittest.main()
