---
name: next-slice
description: Archive the current slice to completed-slices.md and promote the next slice from next-slices.md to current-slice.md
user-invocable: true
---

# Next Slice

Rotate slices: archive the current one and promote the next.

## Steps

1. **Read** `docs/current-slice.md`, and `docs/next-slices.md`.

2. **Archive current slice to completed-slices.md:**
   - Take the content of `docs/current-slice.md` and convert it to a condensed summary matching the style already used in `docs/completed-slices.md`:
     - A heading with the slice name
     - A **short** prose summary of what was built and any key decisions or observations made
         - **Hard cap: 300–400 words total**, including the intro paragraph. This applies regardless of how much work the slice contained — bigger slices mean *more aggressive* trimming, not a longer summary.
         - Each bullet should be ~1–2 sentences. If you find yourself writing `(1) … (2) … (3) …` inside one bullet, that's a sign the bullet is doing too much — pick the headline outcome and drop the rest.
         - Don't preserve investigation detail from the slice doc: specific metric values, dated observations, intermediate hypotheses, file names, and code-level identifiers belong in `current-slice.md` and git history, not the archive. Keep at most one headline number per bullet when it's load-bearing (e.g. "cut R2 ops by 95%").
         - Do mention any new crates introduced / deleted — those are durable architectural facts. Method-level changes are not.
     - No individual checkbox items — summarise by outcome
   - Append this summary to the end of `docs/completed-slices.md`.

3. **Promote the next slice to current-slice.md:**
   - In `docs/next-slices.md`, slices are identified by the `## Slice:` prefix.
   - Identify the slice that is first in the document; this is the next slice.
   - Copy that slice's content **verbatim** (heading and all sub-content) into `docs/current-slice.md`, replacing its entire contents. Use the heading format: `# Current Slice:` followed by the tasks exactly as they appear.
   - **Remove** that slice (heading and all its content, up to the next slice heading or end of file) from `docs/next-slices.md`.

4. **Clean up:** If `docs/next-slices.md` is now empty (only the `# Next Slices` heading remains with no slice content), leave it with just the heading.

5. **Report** what you did: which slice was archived and which is now current.
