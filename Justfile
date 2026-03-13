default:
    just --list

prerequisites:
    brew install protobuf

build:
    cargo build

test:
    cargo test

clippy:
    cargo clippy --workspace -- -D warnings

check: build clippy test

find:
    cargo run --release --bin skeet-finder -- store
