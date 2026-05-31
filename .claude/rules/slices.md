---
paths:
  - "docs/completed-slices.md"
  - "docs/current-slice.md"
---

# Slice Documentation Rules

- **completed-slices.md is append-only history**: never edit existing slice entries. Only add new entries when archiving a completed slice.
- **Checking off a task in current-slice.md**: just flip `[ ]` to `[x]`, leaving the task text as written. Don't append a summary of what you changed or where — the diff and commit history already record that. Add a note only when it conflicts with the task as written (e.g. you did it differently than described) or meaningfully enriches it (a decision or caveat a future reader needs).
