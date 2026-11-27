@echo off
cargo clippy --locked --all-targets --all-features -- -D warnings
