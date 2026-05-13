# BlazeLint 🚀

BlazeLint is a high-performance, industrial-grade linter for Python and Rust, engineered with extreme optimizations. It focuses on speed, low memory overhead, and efficient parallel processing.

## 🛠 Core Technologies & Features

BlazeLint is built on concepts typically reserved for search engines and high-load databases:

* **Memory Mapping (mmap)**: Instead of traditional file reading, we map files directly into the process's address space. This allows the OS to handle caching and makes I/O operations nearly instantaneous.
* **Zero-Copy Parsing**: The linter analyzes raw bytes directly from memory without creating intermediate string objects. This drastically reduces RAM usage and eliminates Garbage Collection (GC) overhead.
* **SIMD-Accelerated Search**: Utilizing the `memchr` crate, BlazeLint leverages CPU vector instructions (SSE/AVX) to scan for newlines and patterns across multiple bytes in a single CPU cycle.
* **Rayon Parallelism**: File processing is automatically distributed across all available CPU cores, enabling the analysis of thousands of files in seconds.
* **Smart Caching**: Includes a built-in caching system (`.blazelint-cache.json`) that tracks file metadata (mtime/size) to skip already processed, unchanged files.

## 🚀 Usage

### Installation
Ensure you have the Rust toolchain installed, then clone the repository and build:
```bash
cargo build --release
```

### Basic check
```bash
cargo run -- check .
```

### Check with automatic fixing
```bash
cargo run -- check . --fix
```

### Ignore cache and force re-scan
```bash
cargo run -- check . --no-cache
```

### Standard benchmark run
```bash
cargo bench
```

### Save current performance as a baseline for future comparison
```bash
cargo bench -- --save-baseline main
```

### Compare current code against the saved baseline
```bash
cargo bench -- --baseline main
```
