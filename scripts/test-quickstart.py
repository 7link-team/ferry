"""Exercise credential retention and real command construction without a daemon."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("quickstart", Path(__file__).with_name("quickstart.py"))
quickstart = importlib.util.module_from_spec(spec)
spec.loader.exec_module(quickstart)


class QuickstartTests(unittest.TestCase):
    def test_repeated_initialization_preserves_every_secret_and_address(self):
        with tempfile.TemporaryDirectory() as directory:
            env = Path(directory) / ".env.production"
            self.assertTrue(quickstart.initialize(env, "postgres", "https://ops.example.com", "127.0.0.1", 4290))
            before = env.read_bytes()
            self.assertFalse(quickstart.initialize(env, "postgres", "http://localhost:1234", "0.0.0.0", 1234))
            self.assertEqual(before, env.read_bytes())
            values = dict(line.split("=", 1) for line in env.read_text().splitlines() if not line.startswith("#"))
            keys = ("FERRY_MASTER_KEY", "DATABASE_PASSWORD", "MYSQL_ROOT_PASSWORD", "POSTGRES_ROOT_PASSWORD", "NATS_PASSWORD", "REDIS_PASSWORD")
            self.assertEqual(len({values[key] for key in keys}), len(keys))
            for key in keys:
                self.assertRegex(values[key], r"^[a-f0-9]{64}$")
            self.assertEqual(values["FERRY_SECURE_COOKIE"], "true")
            with self.assertRaises(ValueError):
                quickstart.initialize(env, "mysql", "http://localhost:4290", "127.0.0.1", 4290)
            self.assertEqual(before, env.read_bytes())

    def test_unsafe_origins_are_rejected_before_writing(self):
        with tempfile.TemporaryDirectory() as directory:
            env = Path(directory) / ".env.production"
            for url in ["file:///tmp/test", "https://user:secret@example.com", "https://example.com/a", "https://example.com?x=1", "https://example.com/#x", "http://example.com/$HOME"]:
                with self.subTest(url=url), self.assertRaises(ValueError):
                    quickstart.initialize(env, "postgres", url, "127.0.0.1", 4290)
                self.assertFalse(env.exists())

    def test_both_database_commands_validate_then_wait_then_read_setup(self):
        for database in ("postgres", "mysql"):
            with self.subTest(database=database), tempfile.TemporaryDirectory() as directory:
                with patch.object(quickstart, "ROOT", Path(directory)), patch.object(quickstart.shutil, "which", return_value="docker"), patch.object(quickstart.subprocess, "run") as run, patch("sys.argv", ["quickstart.py", "--database", database, "--port", "4399"]):
                    quickstart.main()
                calls = run.call_args_list
                self.assertEqual(len(calls), 3)
                self.assertEqual(calls[0].args[0][-2:], ["config", "--quiet"])
                self.assertEqual(calls[1].args[0][-5:], ["up", "-d", "--wait", "--wait-timeout", "300"])
                command = calls[1].args[0]
                self.assertIn(str(Path(directory) / f"compose.{database}.yaml"), command)
                self.assertIn("redis", command)
                self.assertNotIn("shell", calls[1].kwargs)
                self.assertTrue(calls[1].kwargs["check"])
                self.assertIn("FERRY_PUBLIC_URL=http://localhost:4399", (Path(directory) / ".env.production").read_text())

    def test_init_only_does_not_run_docker(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(quickstart, "ROOT", Path(directory)), patch.object(quickstart.subprocess, "run") as run, patch("sys.argv", ["quickstart.py", "--init-only"]):
            quickstart.main()
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
