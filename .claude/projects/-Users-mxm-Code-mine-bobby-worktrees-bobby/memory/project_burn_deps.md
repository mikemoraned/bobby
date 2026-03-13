---
name: Burn dependency pinning
description: burn-import 0.20 requires pinning lzma-rust2 and candle-core to avoid build failures
type: project
---

burn-import ~0.20 has broken transitive dependencies that must be pinned in face-detection/Cargo.toml:
- `lzma-rust2 = "~0.15.7"` — version 0.15.3 is incompatible with crc 3.x
- `candle-core = "=0.9.1"` — version 0.9.2 added DType variants that burn-import 0.20 doesn't handle

Also requires `burn-store = { version = "~0.20", features = ["burnpack"] }` as runtime dep for loading generated model weights.

**Why:** Discovered through trial and error; the working reference project (mgm-bobby) had these pinned in its lockfile.
**How to apply:** If upgrading burn versions, verify these transitive deps still resolve correctly.
