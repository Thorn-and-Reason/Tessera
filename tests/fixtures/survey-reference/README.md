# Synthetic survey reference data

These files contain invented responses and reserved `.invalid` email addresses.
They model a Forms-style export, rather than a claim about every Forms version.
The XLSX contains the same values as `forms.csv`, with typed numeric cells and
an empty measurement cell. The CSV has a whitespace-only measurement cell.

The eight measurements are 1, 2, 3, 4, 5, 3, 3 and missing. Seven valid responses
give a mean of 3, sample variance of 5/3 and sample standard deviation of
sqrt(5/3). “Agree” occurs three times: 37.5% of all eight responses and 300/7%
of valid responses after explicitly defining the empty satisfaction answer as
missing. “N/A” remains a response unless a missing rule explicitly excludes it.
The exact activity labels are Reading (4), Walking (3) and ` reading ` (1).
Multiple-answer strings remain complete cells: Red;Blue (3), Blue (2), Red (1),
Blue;Red (1) and empty (1). They are not individual option counts.

The four-row edge fixture has measurements 1, missing, 3 and missing: mean 2,
sample standard deviation sqrt(2). Its constant numeric column has mean 5 and
standard deviation 0. Its all-missing column is explicitly imported as numeric.
Identifiers exceed exact integer precision in floating point and must remain
text. HTML-like labels exercise escaped report output.

Expected values are literal test references; they are not calculated by the
analysis implementation under test. The XLSX integration test requires the
supervised workbook helper through `TESSERA_WORKBOOK_WORKER` and is run explicitly.
