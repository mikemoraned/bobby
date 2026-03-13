default:
    just --list

prerequisites:
    brew install protobuf

build:
    cargo build

test:
    cargo test

clippy:
    cargo clippy -- -D warnings

check: build clippy test
