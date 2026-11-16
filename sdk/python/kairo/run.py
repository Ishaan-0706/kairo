"""Run management and event recorder interface."""
from __future__ import annotations
from typing import TYPE_CHECKING, Any, Dict, Optional

if TYPE_CHECKING:
    from .client import KairoClient


class Run:
    """Represents an active or finished agent execution run in Kairo."""

    def __init__(self, client: KairoClient, run_id: str, status: str = "running"):
        self.client = client
        self.run_id = run_id
        self.status = status

    def record(self, event_type: str, payload: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        """Record an event to this run's execution log."""
        data = {
            "event_type": event_type,
            "payload": payload or {},
        }
        resp = self.client._request("POST", f"/api/runs/{self.run_id}/events", data)
        return resp

    def record_llm_request(self, model: str, prompt: str, **kwargs: Any) -> Dict[str, Any]:
        """Helper to record an LLM prompt request."""
        payload = {"model": model, "prompt": prompt}
        payload.update(kwargs)
        return self.record("llm.request", payload)

    def record_llm_response(self, model: str, response: str, **kwargs: Any) -> Dict[str, Any]:
        """Helper to record an LLM generation response."""
        payload = {"model": model, "response": response}
        payload.update(kwargs)
        return self.record("llm.response", payload)

    def record_tool_call(self, tool: str, arguments: Dict[str, Any]) -> Dict[str, Any]:
        """Helper to record a tool invocation request."""
        payload = {"tool": tool, "arguments": arguments}
        return self.record("tool.call", payload)

    def record_tool_result(self, tool: str, output: Any) -> Dict[str, Any]:
        """Helper to record a tool execution output."""
        payload = {"tool": tool, "output": output}
        return self.record("tool.result", payload)

    def complete(self, summary: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        """Finalize the run with a completed status."""
        data = {
            "status": "completed",
            "summary": summary or {},
        }
        resp = self.client._request("POST", f"/api/runs/{self.run_id}/finish", data)
        self.status = "completed"
        return resp

    def fail(self, error: Any = None) -> Dict[str, Any]:
        """Finalize the run with a failed status."""
        if isinstance(error, str):
            summary = {"error": error}
        elif isinstance(error, dict):
            summary = error
        elif error is None:
            summary = {"error": "Run failed without specific reason"}
        else:
            summary = {"error": str(error)}

        data = {
            "status": "failed",
            "summary": summary,
        }
        resp = self.client._request("POST", f"/api/runs/{self.run_id}/finish", data)
        self.status = "failed"
        return resp

    def __enter__(self) -> Run:
        return self

    def __exit__(self, exc_type: Any, exc_val: Any, exc_tb: Any) -> bool:
        if exc_type is not None:
            if self.status == "running":
                try:
                    self.fail({"error": str(exc_val), "type": exc_type.__name__})
                except Exception:
                    pass
            return False  # Let exception propagate to user
        else:
            if self.status == "running":
                self.complete({"status": "auto_completed"})
            return True

    def __repr__(self) -> str:
        return f"<Kairo Run id={self.run_id!r} status={self.status!r}>"
