#!/usr/bin/env python3
"""Example: Math Problem Solver Agent using Kairo Python SDK.

This example demonstrates how an AI agent integrates with Kairo:
1. Connects to the Kairo server.
2. Starts an execution run.
3. Records simulated LLM requests and responses.
4. Records tool calls and tool execution results.
5. Finalizes the run with execution metrics.
"""

import os
import sys
import time

# Ensure sdk/python is on sys.path so 'kairo' can be imported without installing
sdk_dir = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
if sdk_dir not in sys.path:
    sys.path.insert(0, sdk_dir)

from kairo import KairoClient, KairoConnectionError, KairoApiError


def simple_calculator(operation: str, a: float, b: float) -> float:
    """A deterministic calculator tool for the agent."""
    if operation == "multiply":
        return a * b
    elif operation == "divide":
        if b == 0:
            raise ValueError("Division by zero")
        return a / b
    elif operation == "add":
        return a + b
    elif operation == "subtract":
        return a - b
    else:
        raise ValueError(f"Unknown operation: {operation}")


def run_math_agent(server_url: str = "http://127.0.0.1:3000"):
    print(f"Connecting to Kairo server at {server_url}...")
    client = KairoClient(base_url=server_url)

    # 1. Verify server health
    health = client.health()
    print(f"Connected to Kairo: status={health.get('status')} version={health.get('version')}")

    # 2. Start an execution run
    print("\nStarting agent run...")
    run = client.create_run(
        agent="math-solver-agent",
        metadata={
            "goal": "Solve (48 * 12) + (360 / 6)",
            "model": "simulated-gpt-4o",
        },
    )
    print(f"  Run ID: {run.run_id}")
    print(f"  Status: {run.status}")

    try:
        # Step 1: Agent receives problem and queries LLM
        prompt = "Compute (48 * 12) + (360 / 6)"
        print(f"\n[Step 1] Sending prompt to LLM: '{prompt}'")
        run.record_llm_request(model="simulated-gpt-4o", prompt=prompt)

        # Simulated LLM decides to call calculator for 48 * 12
        llm_resp_1 = "I need to calculate 48 * 12 first. Calling calculator tool."
        print(f"[Step 2] LLM Response: '{llm_resp_1}'")
        run.record_llm_response(model="simulated-gpt-4o", response=llm_resp_1)

        # Step 2: Execute tool 1 (multiply 48 * 12)
        tool_1_args = {"operation": "multiply", "a": 48, "b": 12}
        print(f"[Step 3] Tool Call: calculator with args {tool_1_args}")
        run.record_tool_call(tool="calculator", arguments=tool_1_args)

        tool_1_result = simple_calculator("multiply", 48, 12)
        print(f"[Step 4] Tool Result: {tool_1_result}")
        run.record_tool_result(tool="calculator", output={"result": tool_1_result})

        # Step 3: LLM decides to call calculator for 360 / 6
        llm_resp_2 = "Now calculate 360 / 6. Calling calculator tool."
        print(f"\n[Step 5] LLM Response: '{llm_resp_2}'")
        run.record_llm_response(model="simulated-gpt-4o", response=llm_resp_2)

        tool_2_args = {"operation": "divide", "a": 360, "b": 6}
        print(f"[Step 6] Tool Call: calculator with args {tool_2_args}")
        run.record_tool_call(tool="calculator", arguments=tool_2_args)

        tool_2_result = simple_calculator("divide", 360, 6)
        print(f"[Step 7] Tool Result: {tool_2_result}")
        run.record_tool_result(tool="calculator", output={"result": tool_2_result})

        # Step 4: Add the two results
        final_sum = simple_calculator("add", tool_1_result, tool_2_result)
        final_answer = f"The final result of (48 * 12) + (360 / 6) is {tool_1_result} + {tool_2_result} = {final_sum}"
        print(f"\n[Step 8] Final Answer: '{final_answer}'")
        run.record_llm_response(model="simulated-gpt-4o", response=final_answer)

        # Step 5: Mark run completed
        summary = {
            "result": final_sum,
            "tool_calls": 2,
            "llm_calls": 3,
            "status": "success",
        }
        run.complete(summary=summary)
        print(f"\nRun '{run.run_id}' successfully completed and recorded!")
        print(f"Inspect execution with: cargo run -- inspect {run.run_id}")

    except Exception as e:
        print(f"Error during agent execution: {e}")
        run.fail(error=str(e))
        raise


if __name__ == "__main__":
    url = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:3000"
    try:
        run_math_agent(url)
    except KairoConnectionError as e:
        print(f"\nConnection Error: {e}", file=sys.stderr)
        print("Make sure the Kairo server is running with: cargo run -- serve", file=sys.stderr)
        sys.exit(1)
    except KairoApiError as e:
        print(f"\nAPI Error: {e} (Details: {e.details})", file=sys.stderr)
        sys.exit(1)
