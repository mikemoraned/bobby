---
name: No bespoke bash in build scripts
description: Avoid complex bash scripts in Justfile/build tooling — prefer simple declarative approaches
type: feedback
---

Do not write complex bash scripts in just targets or build tooling. Keep commands simple and declarative.

**Why:** User strongly dislikes bespoke bash logic in build scripts — it's fragile and hard to maintain.

**How to apply:** When solving build/Docker problems, prefer Docker-native solutions (dockerignore, build args, multi-stage) over shell script workarounds.
