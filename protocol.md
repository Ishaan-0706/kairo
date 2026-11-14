# Master Protocol: Development Rules for Antigravity

Use these rules throughout the project.

1. **Work on only one phase at a time.**
2. **Do not implement future phases unless explicitly instructed.**
3. **Explain every important Rust concept introduced.**
4. **Before changing files, inspect the existing project.**
5. **After changing files, explain which files changed and why.**
6. **Run automated tests and show their results.**
7. **Give separate manual testing instructions.**
8. **Never claim a test passed unless it was actually executed.**
9. **Do not hide compiler warnings or errors.**
10. **Ask before introducing major dependencies or changing architecture.**
11. **Ponytail by Default:** Whenever writing, refactoring, designing, or reviewing code, strictly apply the `ponytail` skill (ladder of simplicity: YAGNI, reuse existing, stdlib/native first, minimal code, fewest files, no unrequested abstractions, code first followed by concise rationale).
12. **Git Branching & Commit Discipline:** Create appropriate feature branches for each phase/task (e.g., `phase-1-event-model`), commit clean atomic changes with clear messages, and push to the remote repository.

> **Important:** Don't let Antigravity generate hundreds of lines of code before you understand the design.

---

# General Behavioral Guidelines

Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.