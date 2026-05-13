BlazeLint — Ultra-fast security & style linter built with Rust.
Uses parallel processing and atomic writes for maximum performance and safety.
## Features: Multi-threaded scan, 
Security check for API keys, 
Smart Fix for debug logs, 
Atomic Save via tempfile
Build: cargo build --release
Check: ./target/release/ultra_fast_analyzer check .
Auto-Fix: ./target/release/ultra_fast_analyzer check . --fix
