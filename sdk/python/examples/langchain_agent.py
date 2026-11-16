#!/usr/bin/env python3
"""LangChain Agent Instrumented with Kairo.

This example demonstrates how to integrate Kairo with LangChain:
1. Automatically intercepts all LLM requests, prompts, responses, tool calls, and outputs.
2. Captures provider-supplied token consumption.
3. Records everything to Kairo without modifying agent logic.
4. Enables offline replay, diffing, checkpointing, and evaluation.

Prerequisites:
  Set your API key in environment:
    export GOOGLE_API_KEY="your-google-ai-api-key"
    # or
    export GROQ_API_KEY="your-groq-api-key"

Usage:
  # 1. Start Kairo server in a separate terminal:
  cargo run -- serve --port 3000

  # 2. Run this LangChain agent:
  python3 sdk/python/examples/langchain_agent.py
"""

import os
import sys

# Ensure sdk/python is on sys.path
sdk_dir = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
if sdk_dir not in sys.path:
    sys.path.insert(0, sdk_dir)

from kairo import KairoClient, KairoCallbackHandler, KairoConnectionError
from langchain_core.tools import tool


@tool
def calculate(expression: str) -> str:
    """Evaluate a mathematical expression safely (e.g. '48 * 12 + 360 / 6')."""
    allowed_chars = set("0123456789+-*/(). ")
    if not all(c in allowed_chars for c in expression):
        return "Error: Invalid characters in expression"
    try:
        # Evaluate safely restricted to math arithmetic
        result = eval(expression, {"__builtins__": None}, {})
        return str(result)
    except Exception as e:
        return f"Calculation error: {e}"


def get_llm():
    """Initializes the preferred LLM based on available API keys."""
    google_key = os.environ.get("GOOGLE_API_KEY") or os.environ.get("GEMINI_API_KEY")
    groq_key = os.environ.get("GROQ_API_KEY")
    openai_key = os.environ.get("OPENAI_API_KEY")

    if google_key:
        print("[LLM Provider] Using Google Gemini (gemini-2.0-flash)")
        from langchain_google_genai import ChatGoogleGenerativeAI
        return ChatGoogleGenerativeAI(model="gemini-2.0-flash", api_key=google_key, temperature=0)

    if groq_key:
        print("[LLM Provider] Using Groq (llama-3.1-8b-instant)")
        from langchain_groq import ChatGroq
        return ChatGroq(model="llama-3.1-8b-instant", api_key=groq_key, temperature=0)

    if openai_key:
        print("[LLM Provider] Using OpenAI (gpt-4o-mini)")
        from langchain_openai import ChatOpenAI
        return ChatOpenAI(model="gpt-4o-mini", api_key=openai_key, temperature=0)

    print("\n[ERROR] No API key found!", file=sys.stderr)
    print("Please set one of the following environment variables:", file=sys.stderr)
    print("  export GOOGLE_API_KEY='your-key'", file=sys.stderr)
    print("  export GROQ_API_KEY='your-key'", file=sys.stderr)
    print("  export OPENAI_API_KEY='your-key'", file=sys.stderr)
    sys.exit(1)


def main():
    server_url = os.environ.get("KAIRO_SERVER_URL", "http://127.0.0.1:3000")
    client = KairoClient(base_url=server_url)

    # 1. Verify Kairo server is reachable
    try:
        health = client.health()
        print(f"[Kairo] Connected to server: status={health.get('status')} version={health.get('version')}")
    except KairoConnectionError as e:
        print(f"[Kairo Error] {e}", file=sys.stderr)
        print("Please start the Kairo server with: cargo run -- serve --port 3000", file=sys.stderr)
        sys.exit(1)

    # 2. Get LLM and bind tools
    llm = get_llm()
    tools = [calculate]
    llm_with_tools = llm.bind_tools(tools)

    # 3. Define problem / goal
    goal = "Calculate (48 * 12) + (360 / 6) using the calculator tool."
    print(f"\n[Agent Goal] {goal}")

    # 4. Start a Kairo Run
    run = client.create_run(
        agent="langchain-agent",
        metadata={"goal": goal, "framework": "langchain"},
    )
    print(f"[Kairo Run Started] Run ID: {run.run_id}")

    # 5. Attach Kairo Callback Handler
    kairo_handler = KairoCallbackHandler(run=run)

    try:
        # Step 1: Initial LLM invocation with tool awareness
        print("\n[Step 1] Querying LLM with available tools...")
        ai_message = llm_with_tools.invoke(goal, config={"callbacks": [kairo_handler]})

        # Step 2: Handle any tool calls requested by the model
        final_answer = ai_message.content
        if ai_message.tool_calls:
            print(f"[Step 2] Model requested tool call: {ai_message.tool_calls}")
            tool_map = {t.name: t for t in tools}
            messages = [("human", goal), ai_message]

            for tool_call in ai_message.tool_calls:
                selected_tool = tool_map[tool_call["name"]]
                tool_output = selected_tool.invoke(tool_call["args"], config={"callbacks": [kairo_handler]})
                print(f"[Step 3] Tool Output: {tool_output}")
                messages.append({
                    "role": "tool",
                    "content": str(tool_output),
                    "tool_call_id": tool_call["id"],
                })

            # Step 3: Final LLM synthesis with tool result
            print("[Step 4] Synthesizing final answer...")
            final_response = llm_with_tools.invoke(messages, config={"callbacks": [kairo_handler]})
            final_answer = final_response.content

        print(f"\n[Final Agent Answer]\n{final_answer}")

        # 6. Mark Run Completed in Kairo
        run.complete(summary={"result": str(final_answer), "status": "success"})
        print(f"\n[Success] Run '{run.run_id}' recorded to Kairo!")
        print(f"Inspect execution with:\n  cargo run -- inspect {run.run_id}")
        print(f"Create an evaluation case with:\n  cargo run -- eval create {run.run_id} --id eval-langchain-math")

    except Exception as e:
        run.fail(error=str(e))
        print(f"[Execution Error] {e}", file=sys.stderr)
        raise


if __name__ == "__main__":
    main()
