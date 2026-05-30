---
name: Use jq for JSON parsing
description: Always use jq for JSON parsing; never use inline python scripts
type: feedback
---

Use `jq` for JSON parsing in bash commands. Never use inline python programs (e.g. `python3 -c "import json..."`) for examining JSON.

**Why:** User preference — jq is the right tool for this.

**How to apply:** Whenever parsing JSON output from commands like `cargo metadata`, use `jq` filters instead of piping to python.
