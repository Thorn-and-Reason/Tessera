# Tessera workbook worker

This crate provides Tessera’s bounded Rust workbook supervisor and isolated Calamine 0.36.1 worker. The application links the supervisor library and launches its packaged sibling worker. Only the worker installs the quota allocator; never install it in the desktop application.

From this directory, run:

```sh
python3 make_fixtures.py
cargo test --offline --locked
cargo clippy --offline --locked --all-targets -- -D warnings
cargo build --offline --locked
./target/debug/tessera-workbook-worker --allocator-self-check
cargo run --offline --locked --example workbook-proof -- ./target/debug/tessera-workbook-worker fixtures/tiny.xlsx
```

Python generates synthetic fixtures only. The supervisor, protocol and worker need no Python runtime. The CLI supervisor selects the first 100 rows and columns of the first worksheet. The library inspects bounded worksheet metadata and selects a worksheet index and rectangle. The desktop application now provides worksheet, rectangle, optional-header and column-type controls, with separate preview and commit jobs.

The supervisor copies bounded source bytes into an owned, read-only temporary snapshot before starting the worker. `WorkbookSnapshot` retains that source across inspection and selection; neither operation reopens the original. A SHA-256 digest identifies the snapshot, and each response must match that digest. Replacing the original after inspection does not affect selection. It reserves source-copy storage, the complete worker heap allowance, bounded returned storage and transport overhead against a 256 MiB aggregate ceiling. Source input remains capped at 128 MiB and selected rectangles at two million cells. Returned records are withheld until the protocol ends cleanly and the worker exits successfully. Invalid output, worker failure, cancellation and timeout discard all records. A bounded pipe reader prevents a child from growing parent memory through output. The supervisor kills and reaps only its owned process before joining the reader.

The allocator accounts requested allocation sizes, including conservative overlap during reallocation. It excludes allocator overhead, stack, executable mappings and total resident memory. The resolved ZIP/deflate feature tree uses the Rust allocator. Linux ARM64 container tests exercise the worker quota and supervision boundary. Native Windows enforcement, temporary-file permissions and graphical workflows remain unverified; Windows checks are cross-compilation only. Timeout covers the owned worker, not blocked operating-system reads while preparing its source snapshot. Temporary-snapshot cleanup is best effort.

## Snapshot and inspection API

`WorkbookSnapshot::prepare(source, retained_bytes, cancel)` returns a cloneable owner with `source_identity()` and `owned_bytes()`. The ownership value conservatively reserves compressed source length, plus path and owner storage, even though the retained source is on disk. Deduplicate clones when counting retained work.

`inspect(&snapshot, &config)` returns `WorkbookInfo` with source identity, workbook epoch and indexed worksheet names, kind, visibility and declared dimension hints. `WorkbookInfo::owned_bytes()` counts its retained metadata. The list has ceilings of 256 sheets, 1 KiB per name and 64 KiB for wire output and retained metadata. Non-worksheet entries are listed without loading VBA content; cell selection refuses them. Dimension hints may default to A1 when the source omits its dimension declaration; they do not prove occupied-cell bounds.

`run_snapshot(&snapshot, &config)` returns selected cells. `Config` supplies the worker path, worksheet index, rectangle, requested-heap quota, retained work, timeout and an `Arc<AtomicBool>` cancellation flag. Its source path is used only by the convenience `run(config)` wrapper. Configured retained bytes exclude this snapshot, which the helper counts itself, but include retained inspection metadata and all other application work. The worker allowance is at least 64 KiB and must fit the remaining aggregate budget. Tests use 2 MiB; application integration can reserve a larger allowance without changing the aggregate ceiling. Timeout must be positive and at most 60 seconds.

The worker receives `--inspect snapshot quota` for metadata or `--protocol snapshot quota worksheet_index row0 column0 row1 column1` for selection. Metadata protocol `XLM1` starts with its four-byte version, 32-byte source digest and one epoch byte. Each marker-0 entry carries index, sheet kind, visibility, a dimension-presence byte, four little-endian u32 coordinates, a little-endian u16 name length and UTF-8 name. Marker 255 ends the stream. The parent validates count, names, dimensions, source identity and complete successful child exit.

## Binary cell protocol XLP2

The worker writes the four ASCII bytes `XLP2`, followed by the 32-byte snapshot digest, a little-endian u32 worksheet index and one workbook-epoch byte. The parent requires its expected source identity and worksheet index. Each cell record has marker `0`, zero-based row and column as little-endian `u32`, one flags byte, one type byte, a little-endian `u32` payload length, then payload bytes. A single `255` byte ends the stream. EOF without that byte, trailing bytes and unsuccessful worker exit all fail.

Types are: `0` missing with no payload; `1` finite Float64 in eight little-endian bytes; `2` UTF-8 text; `3` Excel date/time serial in eight bytes; `4` Excel duration serial in eight bytes. Flags are: `1` formula present, `2` formula without a cached value, `8` 1904 epoch, `16` fictitious 1900-02-29, and `32` numeric source lexeme unavailable. A formula with a cached value has flag 1 without flag 2. Dates retain serial and epoch; `Value::iso_text()` converts them to ISO text at millisecond precision, with no invented timezone, and refuses the fictitious 1900-02-29. Duration records convert separately to signed ISO PT-seconds text. The parent validates flag/type combinations in both directions, finite numbers, date range, UTF-8, strictly increasing coordinates, the selected rectangle and duplicate cells. Every Float64 numeric record must have flag 32; a formula missing its cache must have flags 1 and 2; a 1900-epoch serial on day 60 must have flag 16. Date epoch flags must match the response header.

A text payload is capped at 4 KiB; total wire output and retained record storage each have an 8 MiB ceiling. These are current Excel import limits; CSV capacity is unchanged. Error cells currently fail rather than silently changing values. Booleans become text. ISO date/duration cells remain source text. No formulas, macros or external links execute.

Calamine has already rounded XML numeric lexemes before returning Float64. Every such numeric record carries flag 32. The hostile integer fixture demonstrates that `9007199254740993` becomes `9007199254740992`; this protocol does not recover the original digits. The desktop requires an explicit precision acknowledgement for every column with numeric source values, including columns selected as Text. Text identifiers remain exact; selecting Text cannot recover digits already lost in a numeric source. Cached formula values require a stale-value acknowledgement, and absent caches require a separate missing-value approval.

## Integration and evidence

The desktop uses `InspectExcelPath`, `PreviewExcel` and `CommitExcel` jobs with shared `JobControl` cancellation and retained-work accounting. Workbook and preview owners share the same immutable source snapshot; accounting deduplicates that owner. Generation and cancellation gates control publication. Failed or cancelled work leaves the current project and retained import source available for retry. The packager includes the sibling worker and its dependency notices.

The recorded Excel integration epoch passed 952 top-level application tests and 18 workbook tests: 970 Rust tests in total, excluding two nested fresh-process summaries. Distinct production line coverage was 29,439 / 32,872, or 89.556%. This is an earlier source epoch, before subsequent interface/report work; it is not coverage of every later edit or of Windows-only code. Linux ARM64 executed 18 workbook tests and 28 publication tests. Native Mac observations confirmed tiny-workbook import and project Save/Open, plus second-worksheet preview and cancellation. Windows evidence is cross-compilation only.

The hostile ZIP expansion fixture has a shared-string entry expanding to 16,777,325 bytes from an archive of 17,847 bytes; a 2 MiB worker allowance rejects it. Fixture tests also cover malformed and partial protocols, output floods, timeout/cancellation, source replacement, metadata ceilings, dates, missing formula caches and numeric precision. These checks establish their tested cases, not universal malformed-workbook resistance or a total RSS cap.

Earlier development verification is retained privately by Thorn & Reason. Native Linux graphical workflows, native Windows execution, `.xls` import, AppImage delivery and total-process memory measurement remain unverified or unimplemented.
