#!/usr/bin/env python3
"""Test the SDDM launcher with isolated fake Hyprland and helper processes."""

import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import unittest


ADAPTER = Path(__file__).resolve().parents[1] / "adapters" / "session"
COMMAND = ADAPTER / "veilshell-session"
MOCK = """#!/usr/bin/env python3
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import time

name = Path(sys.argv[0]).name
def log(event):
    with open(os.environ["TEST_LOG"], "a") as file:
        file.write(event + "\\n")
def stop(*_):
    log(name + " stopped")
    sys.exit(0)
signal.signal(signal.SIGTERM, stop)

if name == "Hyprland":
    if "--verify-config" in sys.argv:
        assert Path(os.environ["VEILSHELL_SESSION_CONFIG_DIR"]) == Path(sys.argv[-1]).parent
        sys.exit(int(os.getenv("TEST_VERIFY_STATUS", "0")))
    if os.getenv("TEST_HYPR_FAIL"):
        sys.exit(1)
    assert "DISPLAY" not in os.environ
    assert "HYPRLAND_INSTANCE_SIGNATURE" not in os.environ
    assert os.environ["HYPRLAND_NO_SD_VARS"] == "1"
    assert os.environ["HYPRLAND_NO_SD_TARGET"] == "1"
    assert os.environ["HYPRLAND_NO_SD_NOTIFY"] == "1"
    assert "NOTIFY_SOCKET" not in os.environ
    runtime = Path(os.environ["XDG_RUNTIME_DIR"])
    os.environ.update(WAYLAND_DISPLAY="isolated", HYPRLAND_INSTANCE_SIGNATURE="fake-hyprland")
    ipc = runtime / "hypr" / "fake-hyprland"
    ipc.mkdir(parents=True)
    wl = socket.socket(socket.AF_UNIX)
    hypr = socket.socket(socket.AF_UNIX)
    wl.bind(str(runtime / "isolated"))
    hypr.bind(str(ipc / ".socket.sock"))
    log("Hyprland started")
    subprocess.run(["veilshell-session", "--publish-display"], check=True)
elif name in ("veilshell", "hypridle", "veilshell-autostart"):
    assert os.environ["WAYLAND_DISPLAY"] == "isolated"
    assert os.environ["HYPRLAND_INSTANCE_SIGNATURE"] == "fake-hyprland"
    assert Path(os.environ["VEILSHELL_SOCKET"]).parent == Path(os.environ["VEILSHELL_SESSION_DIR"])
    log(name + " started")
    if name == "veilshell":
        assert " ".join(sys.argv[1:]) == os.getenv("TEST_STARTUP_ARGS", "--policy tablet --scene home")
        if "TEST_SHELL_STATUS" in os.environ:
            time.sleep(0.2)
            sys.exit(int(os.environ["TEST_SHELL_STATUS"]))
    elif name == "hypridle" and os.getenv("TEST_IDLE_FAIL"):
        sys.exit(1)
    elif name == "veilshell-autostart" and "TEST_AUTOSTART_STATUS" in os.environ:
        sys.exit(int(os.environ["TEST_AUTOSTART_STATUS"]))
elif name == "hyprlock":
    assert "--grace" in sys.argv and sys.argv[sys.argv.index("--grace") + 1] == "0"
    log("hyprlock started")
    if os.getenv("TEST_POWER"):
        sys.exit(0)
elif name == "veilshell-lock":
    log("veilshell-lock started")
    if os.getenv("TEST_POWER"):
        sys.exit(0)
elif name == "hyprctl":
    assert sys.argv[1:] == ["locked"]
    print("true" if os.getenv("TEST_LOCKED") else "false")
    sys.exit(0)
elif name == "systemctl":
    assert sys.argv[1:] == ["--no-ask-password", "suspend"]
    log("suspended")
    sys.exit(0)
elif name == "xdg-settings":
    assert sys.argv[1:] == ["get", "default-web-browser"]
    print(os.getenv("TEST_BROWSER", "fixture-browser.desktop"))
    sys.exit(0)
elif name in ("gtk-launch", "xdg-terminal-exec", "xdg-open"):
    assert os.environ["WAYLAND_DISPLAY"] == "isolated"
    assert os.environ["HYPRLAND_INSTANCE_SIGNATURE"] == "private"
    log(" ".join([name, *sys.argv[1:]]))
    sys.exit(0)
else:
    sys.exit(0)

while True:
    time.sleep(0.05)
"""


class SessionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="veil-test-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.runtime = self.root / "runtime"
        self.runtime.mkdir(mode=0o700)
        bindir = self.root / "bin"
        self.bindir = bindir
        bindir.mkdir()
        mock = bindir / "mock"
        mock.write_text(MOCK)
        mock.chmod(0o755)
        for name in (
            "Hyprland", "hyprctl", "hypridle", "hyprlock", "veilshell-lock", "systemctl",
            "veilshell", "veilctl", "xdg-settings", "gtk-launch", "xdg-terminal-exec", "xdg-open",
        ):
            (bindir / name).symlink_to(mock)
        (bindir / COMMAND.name).symlink_to(COMMAND)
        self.log = self.root / "events"
        self.env = dict(
            os.environ,
            PATH=f"{bindir}:{os.environ['PATH']}",
            XDG_RUNTIME_DIR=str(self.runtime),
            XDG_CONFIG_HOME=str(self.root / "config"),
            VEILSHELL_SESSION_CONFIG_DIR=str(ADAPTER),
            TEST_LOG=str(self.log),
            DISPLAY="stale",
            HYPRLAND_INSTANCE_SIGNATURE="stale",
            NOTIFY_SOCKET="stale",
        )
        for key in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET", "VEILSHELL_SESSION_DIR"):
            self.env.pop(key, None)

    def run_launcher(self, *args):
        return subprocess.run(
            [COMMAND, *args], env=self.env, text=True, capture_output=True, timeout=6
        )

    def wait_for(self, event):
        deadline = time.monotonic() + 4
        while time.monotonic() < deadline:
            if self.log.exists() and event in self.log.read_text():
                return
            time.sleep(0.02)
        self.fail(f"missing event: {event}")

    def test_normal_logout_cleans_up(self):
        self.env["TEST_SHELL_STATUS"] = "0"
        result = self.run_launcher()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("hypridle stopped", self.log.read_text())
        self.assertIn("Hyprland stopped", self.log.read_text())
        self.assertEqual(list(self.runtime.glob("veilshell.*")), [])

    def test_saved_startup_mode_is_applied_without_sourcing_config(self):
        prefs = Path(self.env["XDG_CONFIG_HOME"]) / "veilshell"
        prefs.mkdir(parents=True)
        self.env["TEST_SHELL_STATUS"] = "0"
        for index, (value, args) in enumerate((
            ("desk", "--policy laptop --scene pill"),
            ("auto", "--policy auto --scene pill"),
            ("invalid; exit 99", "--policy tablet --scene home"),
        )):
            with self.subTest(value=value):
                runtime = self.root / f"runtime-{index}"
                runtime.mkdir(mode=0o700)
                self.env["XDG_RUNTIME_DIR"] = str(runtime)
                (prefs / "startup.mode").write_text(value + "\n")
                self.env["TEST_STARTUP_ARGS"] = args
                result = self.run_launcher()
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_failed_start_and_shell_map_status_one(self):
        self.env["TEST_HYPR_FAIL"] = "yes"
        self.assertEqual(self.run_launcher().returncode, 2)
        self.env.pop("TEST_HYPR_FAIL")
        self.env["TEST_SHELL_STATUS"] = "1"
        self.assertEqual(self.run_launcher().returncode, 2)

    def test_ends_session_if_idle_service_exits(self):
        self.env["TEST_IDLE_FAIL"] = "yes"
        result = self.run_launcher()
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("hypridle exited", result.stderr)
        self.assertEqual(list(self.runtime.glob("veilshell.*")), [])

    def test_check_is_read_only_and_does_not_remap_status(self):
        self.env["WAYLAND_DISPLAY"] = "existing"
        self.env["TEST_VERIFY_STATUS"] = "1"
        self.assertEqual(self.run_launcher("--check").returncode, 1)
        self.assertEqual(self.run_launcher().returncode, 2)
        self.assertFalse(self.log.exists())

    def test_signal_reaps_children(self):
        with subprocess.Popen([COMMAND], env=self.env) as process:
            try:
                self.wait_for("veilshell started")
                process.send_signal(signal.SIGTERM)
                self.assertEqual(process.wait(timeout=6), 143)
                self.assertIn("veilshell stopped", self.log.read_text())
            finally:
                if process.poll() is None:
                    process.kill()

    def test_optional_autostart_failure_does_not_end_session(self):
        (self.bindir / "veilshell-autostart").symlink_to("mock")
        self.env["TEST_AUTOSTART_STATUS"] = "7"
        with subprocess.Popen([COMMAND], env=self.env) as process:
            try:
                self.wait_for("veilshell-autostart started")
                self.assertIsNone(process.poll())
                process.terminate()
                self.assertEqual(process.wait(timeout=6), 143)
            finally:
                if process.poll() is None:
                    process.kill()

    def test_optional_autostart_stops_on_logout(self):
        (self.bindir / "veilshell-autostart").symlink_to("mock")
        with subprocess.Popen([COMMAND], env=self.env) as process:
            try:
                self.wait_for("veilshell-autostart started")
                process.terminate()
                self.assertEqual(process.wait(timeout=6), 143)
                self.assertIn("veilshell-autostart stopped", self.log.read_text())
            finally:
                if process.poll() is None:
                    process.kill()

    def publish_session(self):
        self.env["WAYLAND_DISPLAY"] = "isolated"
        self.env["HYPRLAND_INSTANCE_SIGNATURE"] = "private"
        session_dir = self.runtime / "veilshell.private"
        session_dir.mkdir()
        self.env["VEILSHELL_SESSION_DIR"] = str(session_dir)
        (session_dir / "display").write_text("isolated\nprivate\n")

    def test_power_helpers_refuse_another_compositor(self):
        self.publish_session()
        self.env["HYPRLAND_INSTANCE_SIGNATURE"] = "omarchy"
        result = self.run_launcher("--power", "suspend")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("display does not match", result.stderr)
        self.assertFalse(self.log.exists())

    def test_app_shortcuts_run_in_the_standalone_environment(self):
        self.publish_session()
        for target, expected in (
            ("terminal", "xdg-terminal-exec"),
            ("browser", "gtk-launch fixture-browser.desktop"),
            ("files", f"xdg-open {self.env['HOME']}"),
        ):
            result = self.run_launcher("--launch", target)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(expected, self.log.read_text().splitlines())

    def test_app_shortcuts_reject_other_displays_and_invalid_desktop_ids(self):
        self.publish_session()
        self.env["TEST_BROWSER"] = "../invalid.desktop"
        result = self.run_launcher("--launch", "browser")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no valid default browser", result.stderr)
        self.env["HYPRLAND_INSTANCE_SIGNATURE"] = "omarchy"
        result = self.run_launcher("--launch", "terminal")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("display does not match", result.stderr)
        self.assertFalse(self.log.exists())

    def test_lock_requests_do_not_spawn_a_second_locker(self):
        self.publish_session()
        with subprocess.Popen([COMMAND, "--lock"], env=self.env) as locker:
            try:
                self.wait_for("hyprlock started")
                self.assertEqual(self.run_launcher("--lock").returncode, 0)
                self.assertEqual(self.log.read_text().count("hyprlock started"), 1)
            finally:
                locker.terminate()

    def test_suspend_requires_compositor_lock(self):
        self.publish_session()
        self.env["TEST_POWER"] = "yes"
        result = self.run_launcher("--power", "suspend")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("did not confirm", result.stderr)
        self.assertNotIn("suspended", self.log.read_text())
        self.env["TEST_LOCKED"] = "yes"
        result = self.run_launcher("--power", "suspend")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("suspended", self.log.read_text())


    def test_native_locker_opt_in_runs_veilshell_lock(self):
        self.publish_session()
        self.env["VEILSHELL_LOCKER"] = "native"
        self.assertEqual(self.run_launcher("--check").returncode, 0)
        with subprocess.Popen([COMMAND, "--lock"], env=self.env) as locker:
            try:
                self.wait_for("veilshell-lock started")
                self.assertEqual(self.run_launcher("--lock").returncode, 0)
                self.assertEqual(self.log.read_text().count("veilshell-lock started"), 1)
            finally:
                locker.terminate()


if __name__ == "__main__":
    unittest.main()
