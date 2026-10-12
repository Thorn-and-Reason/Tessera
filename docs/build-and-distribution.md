# Building Tessera and obtaining official packages

Tessera's source is free under GPL-3.0-only. You may build, modify and
redistribute it, including compiled copies, subject to that licence. Thorn &
Reason plans to sell prepared official packages after release and delivery
acceptance. Source publication does not mean that the current development
candidate is ready for sale.

## Run from source

Install Rust 1.95 or later (the verified development toolchain is 1.97.1) and
your platform's native compiler/linker. macOS needs Xcode Command Line Tools;
Windows needs the C++ build tools for the selected Rust target. Linux needs
the development libraries required by the desktop UI, portal dialogs and
image rendering.

From the repository root:

```sh
cargo fetch --locked
cargo fetch --locked --manifest-path crates/tessera-workbook/Cargo.toml
cargo test --locked --all-features
cargo test --locked --manifest-path crates/tessera-workbook/Cargo.toml
cargo build --locked --release --manifest-path crates/tessera-workbook/Cargo.toml --target-dir target --bin tessera-workbook-worker
cargo run --locked --release
```

The workbook-worker executable must remain beside the application for Excel
import. The commands above use the same `target/release` directory for both.
See [the README](../README.md) for application features. Python helper checks
require Python 3.11+ and `scripts/requirements-dev.txt`. Install those development
dependencies in a virtual environment, then run:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py'
python3 -m unittest discover -s tests/project_io -p 'test_*.py'
```

For Rust formatting and lint checks:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features
```

The desktop feature is enabled by default. Core checks and development
benchmarks can use `--no-default-features`.

## Build distributable packages

The existing `scripts/package.py` prepares a ZIP containing the application,
Excel worker, examples, licences, dependency notices and build-input hashes.
It does not create a setup wizard or disk image.

On macOS, install both Rust targets and fetch the locked dependencies before
using the offline packager:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo fetch --locked
cargo fetch --locked --manifest-path crates/tessera-workbook/Cargo.toml
python3 scripts/package.py --platform macos --output dist/Tessera-macOS.zip
```

For the Windows x64 package, the current packager uses the GNU Rust target.
On Ubuntu, install `mingw-w64`, add `x86_64-pc-windows-gnu`, fetch both locked
dependency graphs as above, then run:

```sh
python3 scripts/package.py --platform windows --output dist/Tessera-Windows.zip
```

The owner has chosen unsigned Mac distribution. macOS packages retain an ad-hoc
signature; these commands do not perform Developer ID signing, notarization or
Windows signing. See [installation instructions](INSTALLING.md) for the expected
security prompts. Platform and downloaded-file acceptance remain in
[production readiness](production-readiness.md).

## Public source and paid downloads

The public repository runs verification only; it does not publish official
executables. Thorn & Reason's private release workflow builds candidate
packages from a full commit identifier using these same packaging scripts.
The planned customer-delivery service will provide official packages from
private website storage after purchase, once release and delivery acceptance
are complete.

Every distributed package must have a retained corresponding-source archive
for its exact build inputs, including the required dependency source. The free
source link belongs beside the installer links, and must not require payment.
Source snapshots are attached to public `source-<commit>` releases; their
versioned archive and checksums must stay available for shipped packages.
Do not substitute the repository's changing `main` branch for an exact source
archive. See [licensing and distribution](LICENSING.md).

Historical verification records contain paths from the original development
machines. They record earlier checks and are not paths required by your build.
