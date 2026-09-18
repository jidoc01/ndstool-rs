# ndstool-rs

Rust reimplementation of the Nintendo DS ROM tool [devkitPro/ndstool](https://github.com/devkitPro/ndstool).

## 1. Why Reinvent the Wheel?

The original `ndstool` does not provide parallel extraction or an incremental creation workflow. I reimplemented its core NDS workflows in Rust, focusing on concurrent extraction and reusing the previous ROM between builds. Rust's concurrency support and modern language features enabled these changes into separate components. See [Benchmarks](#3-benchmarks) for the measured results.

## 2. Features

- Parallel extraction and incremental creation for Nintendo DS ROMs.
- Compatibility with original `ndstool` creation and extraction workflows.

## 3. Benchmarks

I benchmarked extraction and repeated creation using three 256 MiB ROMs: Kingdom Hearts 358/2 Days, Pokémon Black, and Dragon Quest IX. Compared with the original ndstool, parallel extraction was **4.95–7.78× faster**, and optimized full creation was faster in all twelve edit scenarios. Incremental creation was faster than the original in eleven scenarios, but slower for Pokémon Black file additions. These improvements target extracting many assets and frequently rebuilding edited ROMs.

Windows native; medians of three runs or series for extraction/incremental, and five runs for full creation. Parentheses show elapsed-time change versus **ndstool (Original)**; negative means less time.

### 3-A. Extraction — parallel

| ROM | ndstool (Original) | ndstool-rs (Sequential) | ndstool-rs (Parallel) |
|---|---:|---:|---:|
| Kingdom Hearts 358/2 Days | 11,122 ms | 10,085 ms | **2,049 ms** (−81.6%) |
| Pokémon Black | 5,069 ms | 3,051 ms | **1,024 ms** (−79.8%) |
| Dragon Quest IX | 55,444 ms | 51,411 ms | **7,126 ms** (−87.1%) |

I parallelized independent file extraction instead of processing every file sequentially. On these ROMs, this reduced elapsed time by **79.8–87.1%** compared with the original ndstool.

### 3-B. Creation — incremental

Cumulative time for **10 successive edits and builds**, with the same edited inputs for all three builders. Each step adds a 64 KiB file, removes one file, grows one file by 64 KiB, or halves one file. Incremental builds reuse the previous state and the same output path; the first output export is measured separately and excluded here. All three columns were remeasured together using the optimized streaming full builder.

| ROM | Edit | ndstool (Original) | ndstool-rs (Full) | ndstool-rs (Incremental) |
|---|---|---:|---:|---:|
| Kingdom Hearts 358/2 Days | Add | 4,996 ms | 3,031 ms | **1,424 ms** (−71.5%) |
| Kingdom Hearts 358/2 Days | Remove | 4,969 ms | 2,959 ms | **1,366 ms** (−72.5%) |
| Kingdom Hearts 358/2 Days | Grow | 4,983 ms | 2,986 ms | **1,301 ms** (−73.9%) |
| Kingdom Hearts 358/2 Days | Shrink | 5,070 ms | 2,897 ms | **1,281 ms** (−74.7%) |
| Pokémon Black | Add | 2,731 ms | 2,125 ms | **3,452 ms** (+26.4%) |
| Pokémon Black | Remove | 2,772 ms | 2,136 ms | **2,270 ms** (−18.1%) |
| Pokémon Black | Grow | 2,742 ms | 2,145 ms | **2,058 ms** (−24.9%) |
| Pokémon Black | Shrink | 2,708 ms | 2,109 ms | **1,358 ms** (−49.8%) |
| Dragon Quest IX | Add | 15,386 ms | 7,285 ms | **2,164 ms** (−85.9%) |
| Dragon Quest IX | Remove | 14,951 ms | 7,226 ms | **2,141 ms** (−85.7%) |
| Dragon Quest IX | Grow | 15,017 ms | 7,249 ms | **2,089 ms** (−86.1%) |
| Dragon Quest IX | Shrink | 15,039 ms | 7,227 ms | **1,994 ms** (−86.7%) |

For creation, I focused on avoiding repeated payload reads and whole-ROM writes rather than relying only on parallelism. Inspired by [MSVC's incremental linking](https://learn.microsoft.com/en-us/cpp/build/reference/incremental-link-incrementally?view=msvc-170), I retain the previous ROM and update changed regions. Unchanged payloads keep their offsets; a growing file is relocated individually when it no longer fits. This targets frequent edit/build cycles, not just one-off creation.

Optimized full creation reduced cumulative time by **21.8–52.7%** versus the original. Incremental creation was faster than the original in eleven scenarios, but Pokémon Black additions were **26.4% slower**. It also lost to optimized Full for Pokémon Black additions and removals: the first structural edit expands its preserved 256 MiB snapshot/output to 512 MiB. Expansion and synchronization costs remain included; these are measured results, not a guarantee that incremental always wins.

### 3-C. Creation — full

One complete build, without incremental reuse. Five-run medians on the same three edited fixture trees; these are not ten-build totals.

| ROM | ndstool (Original) | ndstool-rs (Full) |
|---|---:|---:|
| Kingdom Hearts 358/2 Days | 501 ms | **303 ms** (−39.5%) |
| Pokémon Black | 279 ms | **211 ms** (−24.5%) |
| Dragon Quest IX | 1,573 ms | **692 ms** (−56.0%) |

Full creation uses buffered streaming. The table uses sequential creation; parallel creation was slower in this experiment.

## 4. License

Original contributions are offered under the [MIT License](LICENSE). Third-party code and data are not relicensed; see [provenance and unresolved distribution requirements](THIRD_PARTY_NOTICES.md). This project began as a Rust porting effort based on [devkitPro/ndstool](https://github.com/devkitPro/ndstool); MIT-only licensing of the complete program has not been established.
