import os
from pathlib import Path
import signal
import subprocess
import sys
import unittest
from unittest.mock import patch

from pi_real_smoke import build_environment, stop_owner


class SmokeFixtureTests(unittest.TestCase):
    def test_environment_excludes_inherited_credentials_and_pi_resources(self):
        with patch.dict(os.environ, {"OPENAI_API_KEY": "unused", "PI_API_KEY": "unused",
                                     "WAAP_PI_MODEL": "external/model", "PI_SESSION_ID": "parent",
                                     "HTTP_PROXY": "http://invalid", "HOME": "/parent"}):
            env = build_environment(Path("/isolated/pi"), Path("/isolated/run"), "/bin/pi")
        for name in ["OPENAI_API_KEY", "PI_API_KEY", "WAAP_PI_MODEL", "PI_SESSION_ID", "HTTP_PROXY"]:
            self.assertNotIn(name, env)
        self.assertEqual(env["HOME"], "/isolated")
        self.assertEqual(env["PI_CODING_AGENT_DIR"], "/isolated/pi")
        self.assertEqual(env["WAAP_PI_BIN"], "/bin/pi")

    def test_cleanup_reaps_owner_that_ignores_termination(self):
        owner = subprocess.Popen(
            [sys.executable, "-c", "import signal, time; signal.signal(signal.SIGTERM, signal.SIG_IGN); "
             "print('ready', flush=True); time.sleep(60)"],
            start_new_session=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            self.assertEqual(owner.stdout.readline().strip(), "ready")
            stop_owner(owner)
            self.assertEqual(owner.returncode, -signal.SIGKILL)
        finally:
            if owner.poll() is None:
                owner.kill()
            owner.communicate()

    def test_cleanup_kills_remaining_group_even_after_owner_exit(self):
        owner = unittest.mock.Mock(pid=123)
        with patch("pi_real_smoke.os.killpg") as kill:
            stop_owner(owner)
        self.assertEqual(kill.call_args_list,
                         [unittest.mock.call(123, signal.SIGTERM), unittest.mock.call(123, signal.SIGKILL)])
        owner.communicate.assert_called_once_with(timeout=5)

    def test_cleanup_tolerates_already_exited_group(self):
        owner = unittest.mock.Mock(pid=123)
        with patch("pi_real_smoke.os.killpg", side_effect=ProcessLookupError):
            stop_owner(owner)
        owner.communicate.assert_called_once_with(timeout=5)


if __name__ == "__main__":
    unittest.main()
