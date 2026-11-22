#!/usr/bin/env python3
"""Test the session-scoped desktop services supervisor with fake binaries."""

import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest


ADAPTER = Path(__file__).resolve().parents[1] / "adapters" / "session"
COMMAND = ADAPTER / "veilshell-services"

MOCK = """#!/usr/bin/env python3
import os
from pathlib import Path
import signal
import sys
import time

name = Path(sys.argv[0]).name
log_path = os.environ["TEST_LOG"]

def log(line):
    with open(log_path, "a", encoding="utf-8") as fh:
        fh.write(line + "\\n")

def stop(*_):
    log(f"{name} stopped")
    sys.exit(0)

signal.signal(signal.SIGTERM, stop)
log(f"{name} started {' '.join(sys.argv[1:])}".strip())
while True:
    time.sleep(0.05)
"""


class ServicesTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="veil-svc-test-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.runtime = self.root / "runtime"
        self.runtime.mkdir(mode=0o700)
        self.session_dir = self.runtime / "veilshell.private"
        self.session_dir.mkdir(mode=0o700)
        (self.session_dir / "display").write_text("isolated\nprivate\n", encoding="utf-8")

        self.bindir = self.root / "bin"
        self.bindir.mkdir()
        mock = self.bindir / "mock"
        mock.write_text(MOCK, encoding="utf-8")
        mock.chmod(0o755)
        for name in (
            "xdg-desktop-portal",
            "xdg-desktop-portal-hyprland",
            "xdg-desktop-portal-gtk",
            "hyprpolkitagent",
            "mako",
        ):
            (self.bindir / name).symlink_to(mock)

        self.log = self.root / "events.log"
        self.config_home = self.root / "config"
        self.config_home.mkdir()
        self.env = dict(
            os.environ,
            HOME=str(self.root / "home"),
            XDG_CONFIG_HOME=str(self.config_home),
            XDG_RUNTIME_DIR=str(self.runtime),
            VEILSHELL_SESSION_DIR=str(self.session_dir),
            WAYLAND_DISPLAY="isolated",
            HYPRLAND_INSTANCE_SIGNATURE="private",
            VEILSHELL_SERVICES_BIN_DIRS=str(self.bindir),
            PATH=f"{self.bindir}:/usr/bin:/bin",
            TEST_LOG=str(self.log),
        )
        self.env.pop("VEILSHELL_SERVICES", None)
        self.env.pop("VEILSHELL_POLKIT_AGENT", None)
        self.env.pop("VEILSHELL_NOTIFICATION_DAEMON", None)

    def wait_for(self, expected: str, timeout: float = 3.0) -> list[str]:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.log.exists():
                lines = self.log.read_text(encoding="utf-8").splitlines()
                if expected in lines:
                    return lines
            time.sleep(0.05)
        contents = self.log.read_text(encoding="utf-8") if self.log.exists() else "<empty>"
        self.fail(f"timed out waiting for {expected!r}; log:\n{contents}")

    def test_refuses_to_run_outside_standalone_session(self):
        env = dict(self.env, HYPRLAND_INSTANCE_SIGNATURE="omarchy")
        res = subprocess.run(
            [sys.executable, str(COMMAND)], env=env, capture_output=True, text=True
        )
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("display does not match", res.stderr)
        self.assertFalse(self.log.exists())

    def test_check_reports_discovered_services(self):
        res = subprocess.run(
            [sys.executable, str(COMMAND), "--check"],
            env=self.env,
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertIn("xdg-desktop-portal-hyprland", res.stdout)
        self.assertIn("xdg-desktop-portal-gtk", res.stdout)
        self.assertIn("xdg-desktop-portal", res.stdout)
        self.assertIn("hyprpolkitagent", res.stdout)
        self.assertIn("mako", res.stdout)

    def test_starts_portals_polkit_notifications_and_tears_down_cleanly(self):
        proc = subprocess.Popen([sys.executable, str(COMMAND)], env=self.env)
        try:
            self.wait_for("xdg-desktop-portal-hyprland started")
            self.wait_for("xdg-desktop-portal-gtk started")
            self.wait_for("xdg-desktop-portal started --replace")
            self.wait_for("hyprpolkitagent started")
            self.wait_for("mako started")
        finally:
            proc.send_signal(signal.SIGTERM)
            proc.wait(timeout=5)

        lines = self.log.read_text(encoding="utf-8").splitlines()
        self.assertLess(
            lines.index("xdg-desktop-portal-hyprland started"),
            lines.index("xdg-desktop-portal started --replace"),
        )
        for name in (
            "xdg-desktop-portal-hyprland",
            "xdg-desktop-portal-gtk",
            "xdg-desktop-portal",
            "hyprpolkitagent",
            "mako",
        ):
            self.assertIn(f"{name} stopped", lines)

    def test_services_conf_selects_subset_and_none_disables(self):
        cfg = self.config_home / "veilshell" / "services.conf"
        cfg.parent.mkdir(parents=True)
        cfg.write_text("# Only polkit\npolkit\n", encoding="utf-8")

        proc = subprocess.Popen([sys.executable, str(COMMAND)], env=self.env)
        try:
            self.wait_for("hyprpolkitagent started")
            time.sleep(0.2)
        finally:
            proc.send_signal(signal.SIGTERM)
            proc.wait(timeout=5)

        lines = self.log.read_text(encoding="utf-8").splitlines()
        self.assertIn("hyprpolkitagent started", lines)
        self.assertNotIn("mako started", lines)
        self.assertNotIn("xdg-desktop-portal started --replace", lines)

        env_none = dict(self.env, VEILSHELL_SERVICES="none")
        res = subprocess.run([sys.executable, str(COMMAND)], env=env_none, check=True)
        self.assertEqual(res.returncode, 0)


if __name__ == "__main__":
    unittest.main()
