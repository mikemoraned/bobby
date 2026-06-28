---
name: complete-slice
description: Archive the current slice to completed-slices.md
user-invocable: true
---

## Steps

1. **Read** `docs/current-slice.md`

2. **Archive current slice to completed-slices.md:**
   - Take the content of `docs/current-slice.md` and convert it to a condensed summary matching the style already used in `docs/completed-slices.md`:
     - A heading with the slice name
     - A **short** prose summary of what was built and any key decisions or observations made
         - **Hard cap: 300–400 words total**, including the intro paragraph. This applies regardless of how much work the slice contained — bigger slices mean *more aggressive* trimming, not a longer summary.
         - Each bullet should be ~1–2 sentences. If you find yourself writing `(1) … (2) … (3) …` inside one bullet, that's a sign the bullet is doing too much — pick the headline outcome and drop the rest.
         - Don't preserve investigation detail from the slice doc: specific metric values, dated observations, intermediate hypotheses, file names, and code-level identifiers belong in `current-slice.md` and git history, not the archive. Keep at most one headline number per bullet when it's load-bearing (e.g. "cut R2 ops by 95%").
         - **Don't reference the slice doc's internal structure** — no "Group 0/1/…", "Phase N", "§N", "H1–H8", or task/PR numbers. These label *where* work sat in the working doc, not *what* was built, and they're meaningless once the slice doc is gone. Summarise by outcome and group bullets by theme, not by the doc's groups.
         - Do mention any new crates introduced / deleted — those are durable architectural facts. Method-level changes are not.
     - No individual checkbox items — summarise by outcome
   - Append this summary to the end of `docs/completed-slices.md`.

3. **Report** what you did: which slice was archived and any other special steps taken not mentioned above
