"""HTTP Client for communicating with the Kairo Rust core API."""
from __future__ import annotations
import json
import urllib.error
import urllib.request
from typing import Any, Dict, Optional

from .exceptions import KairoApiError, KairoConnectionError
from .run import Run


class KairoClient:
    """Client for the Kairo execution recording and versioning server."""

    def __init__(self, base_url: str = "http://127.0.0.1:3000", timeout: float = 5.0):
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    def health(self) -> Dict[str, Any]:
        """Check if the Kairo server is running and healthy."""
        return self._request("GET", "/health")

    def create_run(
        self,
        run_id: Optional[str] = None,
        agent: str = "python-agent",
        metadata: Optional[Dict[str, Any]] = None,
    ) -> Run:
        """Initialize a new agent execution run."""
        payload: Dict[str, Any] = {
            "agent": agent,
            "metadata": metadata or {},
        }
        if run_id:
            payload["run_id"] = run_id

        data = self._request("POST", "/api/runs", payload)
        return Run(client=self, run_id=data["run_id"], status=data.get("status", "running"))

    def get_run(self, run_id: str) -> Dict[str, Any]:
        """Fetch the details and full event history of a run."""
        return self._request("GET", f"/api/runs/{run_id}")

    def _request(
        self,
        method: str,
        path: str,
        data: Optional[Dict[str, Any]] = None,
    ) -> Dict[str, Any]:
        """Perform an HTTP request to the Kairo server."""
        url = f"{self.base_url}{path}"
        headers = {
            "Accept": "application/json",
            "User-Agent": "kairo-python-sdk/0.1.0",
        }
        body_bytes = None
        if data is not None:
            headers["Content-Type"] = "application/json"
            body_bytes = json.dumps(data).encode("utf-8")

        req = urllib.request.Request(url, data=body_bytes, headers=headers, method=method)

        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                resp_bytes = resp.read()
                if not resp_bytes:
                    return {}
                return json.loads(resp_bytes.decode("utf-8"))
        except urllib.error.HTTPError as e:
            # Server responded with 4xx or 5xx
            try:
                err_body = e.read().decode("utf-8", errors="replace")
            finally:
                e.close()
            err_json = {}
            msg = e.reason
            try:
                err_json = json.loads(err_body)
                if "error" in err_json:
                    msg = err_json["error"]
            except Exception:
                pass
            raise KairoApiError(status_code=e.code, message=msg, details=err_json) from e
        except (urllib.error.URLError, ConnectionRefusedError, TimeoutError, OSError) as e:
            # Network failure or server not running
            raise KairoConnectionError(
                f"Failed to connect to Kairo server at '{self.base_url}': {e}"
            ) from e
