# Run the firehose listener
run:
    cargo run

# Build the project
build:
    cargo build

# Run tests
test:
    cargo test

# Run clippy
clippy:
    cargo clippy

# Run tests and clippy
check: test clippy
