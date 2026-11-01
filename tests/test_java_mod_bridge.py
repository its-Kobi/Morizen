import tempfile
import tomllib
import unittest
import zipfile
from pathlib import Path

from tools import java_mod_bridge


class JavaModBridgeToolTests(unittest.TestCase):
    def test_client_feature_is_optional_and_off_by_default(self):
        manifest = tomllib.loads(
            (java_mod_bridge.ROOT / "crates" / "horizon-client" / "Cargo.toml").read_text(encoding="utf-8")
        )
        self.assertNotIn("java-mods", manifest["features"]["default"])
        self.assertEqual(
            manifest["features"]["java-mods"],
            ["dep:morizen-compat", "dep:morizen-jvm"],
        )
        self.assertTrue(manifest["dependencies"]["morizen-compat"]["optional"])
        self.assertTrue(manifest["dependencies"]["morizen-jvm"]["optional"])

    def test_java_major(self):
        self.assertEqual(java_mod_bridge.java_major("25.0.5"), 25)
        self.assertEqual(java_mod_bridge.java_major("17-ea"), 17)
        self.assertIsNone(java_mod_bridge.java_major("not-java"))

    def test_reproducible_jar_has_only_relative_files(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            classes = root / "classes"
            (classes / "dev" / "morizen").mkdir(parents=True)
            (classes / "dev" / "morizen" / "Example.class").write_bytes(b"class")
            first = root / "first.jar"
            second = root / "second.jar"
            java_mod_bridge.zip_tree(first, [classes])
            java_mod_bridge.zip_tree(second, [classes])
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with zipfile.ZipFile(first) as archive:
                self.assertEqual(archive.namelist(), ["dev/morizen/Example.class"])
                self.assertEqual(archive.getinfo(archive.namelist()[0]).date_time, (1980, 1, 1, 0, 0, 0))


if __name__ == "__main__":
    unittest.main()
