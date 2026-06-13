# Promotion is label-only: it repoints the `production` label in config/refine.toml
# at an already-registered model version. No data migration; the k8s image flip is
# a separate manual step (see docs/versioning.md). Reads/writes a local config file
# only — no store, no secrets.

refine-promote-show:
    cargo run --quiet --release --bin promote -- show

refine-promote-set version:
    cargo run --quiet --release --bin promote -- set {{ version }}
