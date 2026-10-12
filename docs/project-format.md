# Tessera project format, version 3

Survey cleaning records use `clean_text` with a source column, new column name,
whitespace choice and explicit replacement pairs. Trimming runs before exact
replacement; unmatched values remain unchanged and missing responses stay
missing. The original column is retained. Earlier readers that do not recognise
`clean_text` reject projects containing these records, even though the project
version remains 3. Keep the original project when sharing with an older reader.

A `.tessera` file is one UTF-8 JSON object with `format: "tessera-project"`. The writer and reader use only integer `version: 3`. Foreign project headers and other versions are rejected.

The authoritative [JSON Schema](project.schema.json) defines the current structure. [sample.tessera](../examples/sample.tessera), [mixed-analysis.tessera](../examples/mixed-analysis.tessera), [exploration.tessera](../examples/exploration.tessera) and [regression-analysis.tessera](../examples/regression-analysis.tessera) demonstrate datasets and captured histories. Numbered reference schemas are structural test fixtures; they do not describe accepted project headers.

Nine top-level fields are required: `format`, `version`, `title`, `dataset`, `runs`, `next_run_id`, `next_column_id`, `preparation` and `provenance`. Object fields may appear in any order. Records are closed objects: unknown fields, duplicate fields, omitted fields, array-shaped records and trailing JSON values are rejected. The contract includes optional `figures` and `figure_presets` collections. Their absence selects the default presentation; individual settings also have documented defaults. Nullable captured-analysis fields remain explicit. JSON Schema cannot detect duplicate keys after a parser discards them; the guarded Rust decoder checks them.

The dataset preserves its revision, row count and ordered columns. Each column retains its ID, numeric or text kind, metadata and flattened values. Metadata preserves names, descriptions, measurement levels, missing rules, value labels and display precision. Every column has the saved row count; an empty column list requires zero rows. IDs are canonical positive decimal strings representing `u64`, preserving identities above 2^53 without rounding. Column IDs and names are unique. Accepted spelling is preserved.

Number columns contain finite IEEE 754 Float64 values or `null`; text columns contain strings or `null`. Null represents missing data. Empty text remains a present value unless a missing rule names it. Encoding and decoding preserve finite Float64 values, including subnormals and negative zero. Display precision never changes stored values. NaN and infinities are rejected.

Each history entry is exactly `{ "id": "1", "analysis": { "describe": { ... } } }`, with a single analysis tag. Supported tags are `describe`, `pearson`, `one_sample_t`, `paired_t`, `independent_t`, `frequencies`, `histogram`, `scatter`, `boxplot`, `regression`, `anova` and `reliability`. Descriptive histories use the same tagged structure as every other family. There is no separate `operation` field. Runs retain increasing IDs, and `next_run_id` exceeds every retained run ID.

Captured requests, results, historical metadata, method provenance and chart geometry remain stored values. Opening validates their structure and cross-field relationships without running an analysis or reconstructing estimates from the current dataset. A current column may have changed type or name since capture; opening preserves the historical snapshot. Shape and consistency checks do not establish numerical authenticity against original observations. Provenance strings are inert: opening never follows URLs, reads referenced paths or executes commands. Project files can contain identifying data and captured metadata.

Input is limited to 128 MiB and nesting depth to 64. The two-million-cell dataset limit and aggregate 256 MiB ownership budget remain enforced. The preflight counts dimensions before typed column allocation; guarded decoding accounts for input, parser scratch, owners and validation work. Encode/decode cancellation uses the caller's job control, including retained-work budget accounting through the existing public APIs. Publication continues through the recovery-aware atomic file layer; changing the JSON header does not relax destination identity, conflict or recovery checks.

## Figure presentation

The current contract includes `figures` (key/settings records) and `figure_presets` (name/settings records). Missing collections default to empty. A key contains the captured run ID, graph kind and variant number; variant 0 is the original presentation. Kinds are Histogram, Scatter, Boxplot, Residual, Qq, Distribution(column ID) or Frequency(column ID). Projects retain at most 1024 figure presentations and 256 presets. Keys must reference an available captured graph; duplicate keys and preset names are rejected.

Settings change presentation only. They never change captured results or histogram bins. Empty text selects automatic wording. Physical dimensions are 40–500 mm, resolution 72–600 dpi, and raster output is bounded to 40 million pixels. Axis limits must be finite and increase. Text fields allow 4096 UTF-8 bytes each; preset names allow 128 bytes. Unknown fields are rejected. `project.schema.json` describes the current shape; the Rust reader also enforces pixel, string-byte and retained-result invariants.

## Preparation and identities

`next_column_id` is a canonical positive decimal `u64` string greater than every active column identity. It never rewinds when content is restored through undo or redo; new columns cannot reuse identities from abandoned branches. Exhaustion fails before adopting data.

`preparation` contains at most 256 immutable records. Each record has canonical `source_revision` and `result_revision` strings and one closed `step`. Source revisions may have gaps but must follow the previous record result; results advance the source and do not exceed the current dataset revision. All named source identities must precede the column watermark. Loading validates these invariants but never executes steps.

A step is one of `compute`, `recode`, `sort`, `filter` or the descriptive string `dataset_edit`. Compute names a new numeric output, two column/finite-constant operands, and `add`, `subtract`, `multiply` or `divide`. Recode names a source, new output, output kind, 1–256 exact typed pairs, and unmatched handling (`preserve` or `missing`). Names allow 1024 UTF-8 bytes; recorded text values allow 4096 bytes. Source missing values, including user-defined missing values, stay missing. Preserve requires the source and output types to match. New output metadata starts with defaults; row operations preserve all existing metadata and raw values.

Sort names a source and descending flag. It keeps ties stable and always places missing values last. Filter keeps rows matching one typed `compare` or a `missing`/`present` test. Both use current missing rules. Comparison numbers must be finite; unlike display rounding, exact recoding uses stored values. Nonfinite arithmetic and zero division fail atomically. These closed steps can be executed only through the validated preparation owner; no script or expression evaluator exists.

Consecutive ordinary edits are coalesced into one `dataset_edit` boundary with the first source and latest result revisions. The saved dataset retains their exact content. This marker describes changes, has no executable payload and does not promise replay of direct edits or availability of the original imported data.
