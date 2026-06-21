# `skeet-store` architecture: ports & adapters

`skeet-store` is laid out as a single-crate [hexagonal / ports-and-adapters]
design. The file tree *is* the architecture: which module a thing lives in is
decided by which way its dependencies point.

## Dependency direction

```
model  ←  ports  ←  adapters/lance  →  adapters/object_store
```

Dependencies point inward. Outer modules know about inner ones; never the
reverse.

- **`model/`** — the boundary data types **owned by the store** that flow across
  the ports (e.g. `ImageRecord`, `ScoresMap`). Pure data, `shared`-only dependencies. 
  The pure cross-crate data types it builds on (e.g `Appraisal`, `Zone`) live in 
  the `shared` crate and are imported from there directly, not re-exported through 
  the store.
- **`ports/`** — the narrow, storage-agnostic traits consumers depend on
  (e.g. `Images`, `Scores`). Public traits only; they exclusively use `model` types.
  A consumer should depend on the narrowest port(s) it actually uses, not on the 
  whole store.
- **`adapters/`** — the adapter implementations of the ports. Everything
  storage-specific lives under here, so `ports`/`model` stay free of LanceDB and
  Arrow.
  - **`adapters/lance/`** — the LanceDB/R2 **adapter**: the concrete type
    (`SkeetStore`) implementing every port, plus specifics of lancedb e.g `open`,
    `schema`, `query` execution, and table `maintenance`. Everything Arrow- or
    LanceDB-shaped lives here and is **private to this module** — the table fields
    are `pub(in crate::adapters::lance)`, so `ports`/`model` cannot even name a
    `lancedb::Table` or an Arrow array.
  - **`adapters/object_store/`** — the R2/SSE-C layer the adapter writes through
    (connection + encryption config in `args`, the OTel operation-counting
    `r2_metrics` wrapper). A separable sibling tied to the external R2 deployment.
- **`observability/`** — cross-cutting OTel gauges (`store_metrics`) and
  structured query-plan logging (`query_log`). Sits on top of the adapter: it
  observes it.
- **`versioned_cache`**, **`health`**, **`error`** — small cross-cutting pieces
  used across the layers.

## Why

The carve lets each consumer depend only on the capability it needs, and keeps
the storage backend swappable: because nothing outside `lance/` can name a
LanceDB or Arrow type, an alternative adapter can be slotted in behind the same
ports without touching consumers.

## References

- [hexagonal / ports-and-adapters]: Alistair Cockburn, *Hexagonal Architecture*.
- *Master Hexagonal Architecture in Rust* — https://www.howtocodeit.com/articles/master-hexagonal-architecture-rust
  (Rust-specific treatment of ports, adapters, and dependency inversion).

[hexagonal / ports-and-adapters]: https://alistair.cockburn.us/hexagonal-architecture/
