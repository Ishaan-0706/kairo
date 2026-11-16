"""Automated tests for Kairo Python SDK and HTTP Server."""
from __future__ import annotations

import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import unittest

# Ensure sdk/python is on sys.path
sdk_dir = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
if sdk_dir not in sys.path:
    sys.path.insert(0, sdk_dir)

from kairo import KairoClient, KairoConnectionError, KairoApiError, Run


def find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class TestKairoPythonSDK(unittest.TestCase):
    server_process: subprocess.Popen | None = None
    temp_dir: str = ""
    port: int = 0
    base_url: str = ""
    client: KairoClient

    @classmethod
    def setUpClass(cls):
        # 1. Ensure kairo binary is compiled
        repo_root = os.path.abspath(os.path.join(os.path.dirname(__file__), "../../.."))
        subprocess.run(["cargo", "build"], cwd=repo_root, check=True, stdout=subprocess.DEVNULL)
        binary_path = os.path.join(repo_root, "target", "debug", "kairo")

        cls.temp_dir = tempfile.mkdtemp(prefix="kairo_sdk_test_")
        cls.port = find_free_port()
        cls.base_url = f"http://127.0.0.1:{cls.port}"

        # 2. Launch Kairo HTTP server process
        cls.server_process = subprocess.Popen(
            [binary_path, "--dir", cls.temp_dir, "serve", "--port", str(cls.port)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

        cls.client = KairoClient(base_url=cls.base_url, timeout=3.0)

        # 3. Wait for server to become responsive
        started = False
        for _ in range(50):
            try:
                res = cls.client.health()
                if res.get("status") == "ok":
                    started = True
                    break
            except Exception:
                time.sleep(0.05)

        if not started:
            raise RuntimeError(f"Kairo server failed to start on port {cls.port}")

    @classmethod
    def tearDownClass(cls):
        if cls.server_process:
            cls.server_process.terminate()
            cls.server_process.wait(timeout=5)
        if os.path.exists(cls.temp_dir):
            shutil.rmtree(cls.temp_dir, ignore_errors=True)

    def test_01_health_check(self):
        health = self.client.health()
        self.assertEqual(health.get("status"), "ok")
        self.assertEqual(health.get("service"), "kairo")
        self.assertEqual(health.get("version"), "0.1.0")

    def test_02_create_record_and_complete_run(self):
        run = self.client.create_run(
            agent="test-sdk-agent",
            metadata={"env": "testing"},
        )
        self.assertTrue(run.run_id.startswith("run-"))
        self.assertEqual(run.status, "running")

        # Record LLM request and response
        ev1 = run.record_llm_request(model="mock-gpt", prompt="Hello agent")
        self.assertEqual(ev1.get("status"), "recorded")
        self.assertEqual(ev1.get("event_type"), "llm.request")

        ev2 = run.record_llm_response(model="mock-gpt", response="Hello user")
        self.assertEqual(ev2.get("status"), "recorded")

        # Record tool invocation and output
        ev3 = run.record_tool_call(tool="shell", arguments={"cmd": "echo test"})
        self.assertEqual(ev3.get("status"), "recorded")

        ev4 = run.record_tool_result(tool="shell", output="test\n")
        self.assertEqual(ev4.get("status"), "recorded")

        # Complete run
        fin = run.complete(summary={"total_steps": 4})
        self.assertEqual(fin.get("status"), "completed")
        self.assertEqual(run.status, "completed")

        # Fetch run details from API and verify event log
        run_data = self.client.get_run(run.run_id)
        self.assertEqual(run_data["run_id"], run.run_id)
        self.assertEqual(run_data["status"], "completed")
        # 1 start event + 4 recorded events + 1 completion event = 6 total events
        self.assertEqual(run_data["event_count"], 6)

    def test_03_fail_run(self):
        run = self.client.create_run(agent="failing-agent")
        run.record("task.started", {"step": 1})
        fin = run.fail(error="Unexpected tool failure")
        self.assertEqual(fin.get("status"), "failed")
        self.assertEqual(run.status, "failed")

        run_data = self.client.get_run(run.run_id)
        self.assertEqual(run_data["status"], "failed")

    def test_04_invalid_lifecycle_transition_returns_409(self):
        run = self.client.create_run(agent="conflict-agent")
        run.complete({"result": "done"})

        # Recording to an already completed run must return 409 Conflict
        with self.assertRaises(KairoApiError) as cm:
            run.record("tool.call", {"tool": "late_call"})
        self.assertEqual(cm.exception.status_code, 409)
        self.assertIn("Cannot record event", cm.exception.message)

        # Attempting to finish an already completed run must return 409 Conflict
        with self.assertRaises(KairoApiError) as cm:
            run.complete()
        self.assertEqual(cm.exception.status_code, 409)

    def test_05_run_not_found_returns_404(self):
        with self.assertRaises(KairoApiError) as cm:
            self.client.get_run("run-non-existent-99999")
        self.assertEqual(cm.exception.status_code, 404)
        self.assertIn("not found", cm.exception.message)

    def test_06_connection_failure_when_server_unavailable(self):
        dead_port = find_free_port()
        bad_client = KairoClient(base_url=f"http://127.0.0.1:{dead_port}", timeout=0.5)
        with self.assertRaises(KairoConnectionError):
            bad_client.health()

    def test_07_context_manager_auto_complete(self):
        run_id = None
        with self.client.create_run(agent="ctx-agent") as run:
            run_id = run.run_id
            run.record("action", {"data": 123})
        
        # When exiting with block normally, run is auto-completed
        data = self.client.get_run(run_id)
        self.assertEqual(data["status"], "completed")

    def test_08_context_manager_auto_fail_on_exception(self):
        run_id = None
        try:
            with self.client.create_run(agent="ctx-fail-agent") as run:
                run_id = run.run_id
                run.record("action", {"data": "before crash"})
                raise ValueError("Simulated agent runtime crash")
        except ValueError:
            pass

        data = self.client.get_run(run_id)
        self.assertEqual(data["status"], "failed")


if __name__ == "__main__":
    unittest.main()
