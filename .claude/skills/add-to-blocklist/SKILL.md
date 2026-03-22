---
name: add-to-blocklist
description: Block a skeet by image ID or AT URI, looking it up from the fallback or remote store as needed
user-invocable: true
---

# Add to Blocklist

Block a skeet from appearing in the feed.

## Steps

1. **Ask the user** which store to use: `fallback` or `remote` (R2).

2. **Ask the user** what they want to block. This can be either:
   - An **AT URI** (starts with `at://`)
   - An **image ID** (anything that isn't at AT URI)

   Determine which it is by checking whether the input starts with `at://`.

3. **Ask the user** why they want it blocked. Remember this as the `reason`.

4. **Resolve the AT URI** if the user provided an image ID:
   - If `fallback` store: run `just image-metadata-dump-fallback <image_id>` to get the skeet's AT URI
   - If `remote` store: run `just image-metadata-dump-r2 <image_id>` to get the skeet's AT URI
   - Extract the `skeet_id` (AT URI) from the output

   If the user already provided an AT URI, skip this step.

5. **Run the blocklist command:**
   ```
   just add-to-blocklist "<at_uri>" "<reason>"
   ```

6. **Report** the result to the user: which AT URI was blocked and why.
