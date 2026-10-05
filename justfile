set positional-arguments

default:
    @just --list

# Format, lint, and test
check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

fmt:
    cargo fmt

build:
    cargo build --release

run *args:
    cargo run -- "$@"

install:
    cargo install --path . --locked
