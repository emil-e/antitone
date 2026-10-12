# LibRaw development and bounded parallelism

Status: accepted.

## Choice and rationale

Use pinned LibRaw 0.22.0 black subtraction, one roll-fixed WB tuple and AHD development. Retrieve linear developed camera RGB16 before standard-space output matrix/curve clipping. A mature decoder/developer avoids the cost and uncertainty of a custom demosaicer or camera-specific repair. Automatic brightness and per-photo WB substitution would make shared calibration inconsistent.

Enable upstream OpenMP with checksum-pinned, source-built LLVM OpenMP 22.1.8 in checkout-local storage. Building arm64 for macOS 14 makes the runtime reproducible without relying on a system/Homebrew library. The dynamic runtime and license notices must be bundled for distribution.

Bound decode/process calls by priority, CPU availability and memory pressure; scope/restore OpenMP task settings and let idle workers sleep. Foreground/background caps are eight/two threads, reserving two logical CPUs where possible. These are job budgets, not a whole-roll RAW cache.

Decode once and compute whole-sensor statistics only on explicit request. Regional diagnostics use original CFA, margins and black/white levels after development; developed-RGB clipping is not a substitute.

## Constraints

Thread policy must preserve complete RGB16 output, metadata and immutable diagnostics. Differing pixels cannot silently share development/cache identity. Changes to source support or development settings require compatibility and numerical validation. LibRaw calls cannot be interrupted mid-phase; native deployment targets do not establish actual macOS-14 acceptance.

[Source contract](../contracts/processing-model.md#camera-source-representation) · [Diagnostic guide](../native-development.md)

Implementation: `native/raw_bridge.cpp`, `src/raw.rs`, `src/scan.rs`.
