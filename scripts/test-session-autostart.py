#!/usr/bin/env python3
"""Exercise standalone autostart with temporary XDG dirs and live child processes."""

import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import unittest


COMMAND = Path(__file__).resolve().parents[1] / "adapters/session/veilshell-autostart"
APP = """#!/usr/bin/env python3
import json
import os
import signal
import sys

def log(event):
    with open(os.environ["TEST_LOG"], "a") as file:
        file.write(json.dumps(event) + "\\n")

if sys.argv[1] == "tree":
    if os.fork():
        sys.exit(0)
    signal.signal(signal.SIGTERM, lambda *_: (log(["stopped", os.getpid()]), sys.exit(0)))
elif sys.argv[1] == "ignore-term":
    signal.signal(signal.SIGTERM, signal.SIG_IGN)

log([sys.argv[1], os.getpid(), os.getcwd(), os.environ["WAYLAND_DISPLAY"],
     os.environ["HYPRLAND_INSTANCE_SIGNATURE"], sys.argv[2:]])
while True:
    signal.pause()
"""


class AutostartTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix="veil-autostart-")
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.runtime = self.root / "run"
        self.session = self.runtime / "veilshell.test"
        self.session.mkdir(parents=True, mode=0o700)
        (self.session / "display").write_text("wayland-test\nhypr-test\n")
        ipc = self.runtime / "hypr/hypr-test"
        ipc.mkdir(parents=True)
        for endpoint in (self.runtime / "wayland-test", ipc / ".socket.sock"):
            server = socket.socket(socket.AF_UNIX)
            self.addCleanup(server.close)
            server.bind(str(endpoint))
            server.listen()
        self.home, self.user, self.system, self.second = (
            self.root / name for name in ("home", "config", "system", "second")
        )
        self.app = self.root / "test app"
        self.app.write_text(APP)
        self.app.chmod(0o755)
        self.log = self.root / "events"
        self.env = dict(
            os.environ,
            HOME=str(self.home),
            XDG_CONFIG_HOME=str(self.user),
            XDG_CONFIG_DIRS=f"{self.system}:{self.second}",
            XDG_RUNTIME_DIR=str(self.runtime),
            XDG_CURRENT_DESKTOP="Hyprland",
            VEILSHELL_SESSION_DIR=str(self.session),
            WAYLAND_DISPLAY="wayland-test",
            HYPRLAND_INSTANCE_SIGNATURE="hypr-test",
            DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent",
            TEST_LOG=str(self.log),
        )

    def desktop(self, name, args, *, directory=None, extra=""):
        path = (directory or self.user) / "autostart" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            '[Desktop Entry]\nType=Application\nName=Test App\n'
            f'Exec="{self.app}" {args}\n{extra}'
        )
        return path

    def approve(self, *names, directory=None):
        path = (directory or self.user) / "veilshell/autostart.list"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(names))

    def run_once(self):
        return subprocess.run([COMMAND], env=self.env, capture_output=True, text=True, timeout=3)

    def start(self):
        process = subprocess.Popen([COMMAND], env=self.env)
        self.addCleanup(self.stop, process)
        return process

    def stop(self, process):
        if process.poll() is None:
            process.terminate()
        self.assertEqual(process.wait(timeout=3), 0)

    def events(self):
        if not self.log.exists():
            return []
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def wait_for(self, event):
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            for record in self.events():
                if record[0] == event:
                    return record
            time.sleep(0.02)
        self.fail(f"missing event: {event}; got {self.events()}")

    def test_opt_in_and_valid_display_required(self):
        self.desktop("blocked.desktop", "blocked")
        result = self.run_once()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.approve("blocked.desktop")
        self.env["WAYLAND_DISPLAY"] = "other-desktop"
        result = self.run_once()
        self.assertEqual(result.returncode, 1)
        self.assertIn("standalone display does not match", result.stderr)
        self.env["WAYLAND_DISPLAY"] = "wayland-test"
        (self.runtime / "wayland-test").unlink()
        result = self.run_once()
        self.assertEqual(result.returncode, 1)
        self.assertIn("standalone display does not match", result.stderr)
        self.assertEqual(self.events(), [])

    def test_xdg_precedence_and_filters(self):
        self.desktop("ok.desktop", "system", directory=self.system)
        self.desktop("ok.desktop", "second", directory=self.second)
        self.desktop(
            "ok.desktop", "user", extra="OnlyShowIn=VeilShell;\nNoDisplay=true\nTryExec=python3\n"
        )
        for name, key in (
            ("hidden", "Hidden=true"),
            ("not-here", "NotShowIn=VeilShell;"),
            ("only-elsewhere", "OnlyShowIn=KDE;"),
            ("missing-executable", "TryExec=veilshell-nonexistent-executable"),
            ("disabled", "X-GNOME-Autostart-enabled=false"),
            ("activation", "DBusActivatable=true"),
            ("condition", "AutostartCondition=unless-exists testfile"),
            ("terminal", "Terminal=true"),
        ):
            self.desktop(f"{name}.desktop", name, directory=self.system)
            self.desktop(f"{name}.desktop", name, extra=f"{key}\n")
        self.desktop("malformed.desktop", "malformed", directory=self.system)
        self.desktop("malformed.desktop", "").write_text("not a keyfile")
        self.desktop("bad-field.desktop", "bad-field --title=%c")
        self.desktop("fallback.desktop", "fallback", directory=self.second)
        self.desktop("unlisted.desktop", "unlisted", directory=self.second)
        self.approve("# comment", "", "  ok.desktop  ", "ok.desktop", "../ok.desktop",
                     *(path.name for path in (self.user / "autostart").glob("*.desktop")),
                     "fallback.desktop")
        helper = self.start()
        self.wait_for("user")
        self.wait_for("fallback")
        self.stop(helper)
        self.assertCountEqual([e[0] for e in self.events()], ["user", "fallback"])

    def test_exec_expansion_and_duplicate_helper(self):
        self.env["XDG_CONFIG_HOME"] = "invalid-relative"
        home_config = self.home / ".config"
        literal = f"$(touch {self.root / 'should-not-exist'})"
        path = self.desktop(
            "run.desktop", f'run "two words" %F %U %f %u %c %i %k 50%% "{literal}"',
            directory=home_config, extra=f"Icon=test-icon\nPath={self.root}\n",
        )
        self.approve("run.desktop", directory=home_config)
        helper = self.start()
        record = self.wait_for("run")
        second_helper = self.run_once()
        self.assertEqual(second_helper.returncode, 0, second_helper.stderr)
        self.stop(helper)
        self.assertEqual(record[2:], [
            str(self.root), "wayland-test", "hypr-test",
            ["two words", "Test App", "--icon", "test-icon", str(path), "50%", literal],
        ])
        self.assertFalse((self.root / "should-not-exist").exists())
        self.assertEqual(len(self.events()), 1)

    def test_teardown_reaches_descendants_and_bounds_stubborn_children(self):
        self.desktop("tree.desktop", "tree")
        self.desktop("stubborn.desktop", "ignore-term")
        self.approve("tree.desktop", "stubborn.desktop")
        helper = self.start()
        descendant = self.wait_for("tree")[1]
        stubborn = self.wait_for("ignore-term")[1]
        self.stop(helper)
        self.assertEqual(self.wait_for("stopped")[1], descendant)
        self.assertFalse(Path(f"/proc/{stubborn}").exists())


if __name__ == "__main__":
    unittest.main()
