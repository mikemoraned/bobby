---
name: Hash struct contents canonically, not file bytes
description: For drift-detection hashes over serialised configs, hash struct fields via std::hash::Hash + DefaultHasher — never raw file bytes, and don't reach for sha2/length-prefixes by default.
type: feedback
---

For "did this config change" hashes (e.g. `EvalSplit::content_hash`, `RefineModel::version`):

1. **Hash the struct, not the file bytes.** Raw bytes change with formatter version, whitespace, key order, comments, line endings — none of which are real changes.

2. **Use `std::hash::DefaultHasher` + `Hash` trait, not `sha2`/SHA-256.** The use case here is detecting *accidental* change, not resisting an attacker. `RefineModel::version()` in `skeet-refine/src/model.rs` is the project's established pattern: `DefaultHasher::new()`, call `.hash()` on each field, format the `u64` as hex. No external crypto dep needed.

3. **`std::hash::Hash` already handles aliasing.** `Hash for str` writes a 0xff terminator after the bytes; `Hash for Vec<T>` length-prefixes. So `vec!["ab","c"]` and `vec!["a","bc"]` already hash distinctly — no need to wrap items in your own length prefixes.

**Shape to follow:**
```rust
pub fn content_hash(&self) -> String {
    let mut hasher = DefaultHasher::new();
    self.field_a.hash(&mut hasher);
    self.field_b.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
```

Convert non-Hash types (e.g. `DateTime<Utc>`) to a stable representation (`.timestamp_micros()`) before hashing.
