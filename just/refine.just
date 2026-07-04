# Refine: model promotion, training, scoring runs, and evaluation.

# Promotion is label-only: it repoints the `production` label in config/refine.toml
# at an already-registered model version. No data migration; the k8s image flip is
# a separate manual step (see docs/versioning.md). Reads/writes a local config file
# only — no store, no secrets.

refine-promote-show:
    cargo run --quiet --release --bin promote -- show

refine-promote-set version:
    cargo run --quiet --release --bin promote -- set {{ version }}

train purpose model="gpt-4o":
    op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin train -- --store-path {{ R2_STORE }} --purpose "{{ purpose }}" --model "{{ model }}"

# Variance-aware replication: run `train` at three seeds, each candidate written
# to its own scratch copy of refine.toml so the production config is left
# untouched. The baseline is auto-selected as the best-F1 eval of the production
# model on this split (stable across runs — candidates carry new model_versions,
# so they can't be picked as baseline). All runs append to config/eval-results.toml;
# decide across the three afterwards (majority gate + median recall). The scratch
# copy of a passing run is a complete, deployable refine.toml.
train-replicated purpose model="gpt-4o":
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p /tmp/bobby-retrain
    for seed in 42 43 44; do
        echo "=== train seed=$seed ==="
        cp config/refine.toml /tmp/bobby-retrain/refine-seed-$seed.toml
        op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin train -- \
            --store-path {{ R2_STORE }} \
            --purpose "{{ purpose }} (seed=$seed)" \
            --model "{{ model }}" \
            --seed "$seed" \
            --model-output /tmp/bobby-retrain/refine-seed-$seed.toml
    done

refine-eval purpose split_label="default":
    op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin refine-eval -- --store-path {{ R2_STORE }} --purpose "{{ purpose }}" --split-label {{ split_label }}

sample-costs sample_size="10":
    op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin sample-costs -- --store-path {{ R2_STORE }} --sample-size {{ sample_size }}

live-refine:
    op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin live-refine -- --store-path {{ STORE }}

live-refine-r2:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin live-refine -- --store-path {{ R2_STORE }}

live-refine-r2-debug:
    RUST_LOG=skeet_store=debug {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin live-refine -- --store-path {{ R2_STORE }}

classify-examples:
    cargo run --quiet --release --bin classify-examples

eval:
    cargo run --quiet --release --bin eval -- --config-path config/prune.toml --store-path {{ STORE }}

eval-r2:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin eval -- --config-path config/prune.toml --store-path {{ R2_STORE }}

eval-r2-text:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin eval -- --config-path config/prune.toml --store-path {{ R2_STORE }} --categories Face,Metadata,Text
