---
name: Use uv for Python
description: Always use uv for Python dependency management and package installation, never pip
type: feedback
---

Always manage Python dependencies and install packages using `uv`, never `pip` or `pip3`.

**Why:** User has standardized on uv for Python tooling.
**How to apply:** Use `uv add` for dependencies, `uv run` for scripts, `uvx` for one-off tools.
