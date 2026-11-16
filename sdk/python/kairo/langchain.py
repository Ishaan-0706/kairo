"""LangChain Callback Integration for Kairo.

Automatically records all LangChain LLM prompts, generations, tool calls,
tool outputs, errors, and token consumption to a Kairo execution run.
"""

from __future__ import annotations

import json
from typing import Any, Dict, List, Optional

try:
    from langchain_core.callbacks.base import BaseCallbackHandler
    from langchain_core.messages import BaseMessage
    from langchain_core.outputs import LLMResult
    HAS_LANGCHAIN = True
except ImportError:
    HAS_LANGCHAIN = False
    BaseCallbackHandler = object  # type: ignore

from .run import Run


class KairoCallbackHandler(BaseCallbackHandler):
    """LangChain callback handler that streams events to Kairo."""

    def __init__(self, run: Run, auto_finish: bool = False):
        if not HAS_LANGCHAIN:
            raise ImportError(
                "langchain-core is required to use KairoCallbackHandler. "
                "Install it with: pip install langchain-core"
            )
        super().__init__()
        self.run = run
        self.auto_finish = auto_finish
        self._current_tool_name = "tool"

    def on_llm_start(
        self,
        serialized: Dict[str, Any],
        prompts: List[str],
        *,
        invocation_params: Optional[Dict[str, Any]] = None,
        **kwargs: Any,
    ) -> None:
        model = self._extract_model_name(serialized, invocation_params)
        for p in prompts:
            self.run.record_llm_request(model=model, prompt=p)

    def on_chat_model_start(
        self,
        serialized: Dict[str, Any],
        messages: List[List[Any]],
        *,
        invocation_params: Optional[Dict[str, Any]] = None,
        **kwargs: Any,
    ) -> None:
        model = self._extract_model_name(serialized, invocation_params)
        for msg_list in messages:
            formatted = []
            for m in msg_list:
                role = getattr(m, "type", "message")
                content = getattr(m, "content", str(m))
                formatted.append({"role": role, "content": content})

            self.run.record("llm.request", {
                "model": model,
                "messages": formatted,
            })

    def on_llm_end(self, response: Any, **kwargs: Any) -> None:
        usage = None
        if hasattr(response, "llm_output") and isinstance(response.llm_output, dict):
            usage = response.llm_output.get("token_usage") or response.llm_output.get("usage")

        # Extract text responses
        text_responses = []
        if hasattr(response, "generations"):
            for gen_list in response.generations:
                for gen in gen_list:
                    if hasattr(gen, "text") and gen.text:
                        text_responses.append(gen.text)
                    elif hasattr(gen, "message") and hasattr(gen.message, "content"):
                        text_responses.append(str(gen.message.content))

        resp_text = "\n".join(text_responses) if text_responses else ""
        payload: Dict[str, Any] = {"response": resp_text}
        if usage:
            payload["usage"] = usage

        self.run.record("llm.response", payload)

    def on_llm_error(self, error: BaseException, **kwargs: Any) -> None:
        self.run.record("llm.error", {"error": str(error)})

    def on_tool_start(
        self,
        serialized: Dict[str, Any],
        input_str: str,
        **kwargs: Any,
    ) -> None:
        name = serialized.get("name") or kwargs.get("name") or "tool"
        self._current_tool_name = name

        args = input_str
        if isinstance(input_str, str):
            try:
                args = json.loads(input_str)
            except Exception:
                args = {"input": input_str}

        self.run.record_tool_call(tool=name, arguments=args if isinstance(args, dict) else {"input": args})

    def on_tool_end(self, output: Any, **kwargs: Any) -> None:
        name = kwargs.get("name", self._current_tool_name)
        out = output
        if hasattr(output, "content"):
            out = output.content
        self.run.record_tool_result(tool=name, output=out)

    def on_tool_error(self, error: BaseException, **kwargs: Any) -> None:
        name = kwargs.get("name", self._current_tool_name)
        self.run.record("tool.failed", {"tool": name, "error": str(error)})

    def on_chain_error(self, error: BaseException, **kwargs: Any) -> None:
        if self.auto_finish and self.run.status == "running":
            self.run.fail(error=str(error))

    def _extract_model_name(
        self,
        serialized: Dict[str, Any],
        invocation_params: Optional[Dict[str, Any]],
    ) -> str:
        if invocation_params and "model" in invocation_params:
            return str(invocation_params["model"])
        if invocation_params and "model_name" in invocation_params:
            return str(invocation_params["model_name"])
        if serialized and "id" in serialized and len(serialized["id"]) > 0:
            return str(serialized["id"][-1])
        return "unknown-llm"
