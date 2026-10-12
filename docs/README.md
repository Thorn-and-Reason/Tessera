# Documentation index

- [User guide](using-tessera.md): import, survey reports, preparation, analysis,
  figures, saving, recovery and keyboard navigation.
- [Desktop design system](design/desktop-design-system.md): interface design
  and its recorded verification.
- [Project format](project-format.md) and [JSON Schema](project.schema.json):
  the accepted `.tessera` header is `tessera-project`, version 3.
- [Licensing and distribution](LICENSING.md): GPL-3.0-only terms and notices.
- [Build and distribution instructions](build-and-distribution.md): source builds,
  package preparation and exact corresponding-source access.
- [Installation instructions](INSTALLING.md): unsigned Mac packages and platform limits.
- [Production-readiness requirements](production-readiness.md): release gates and
  the native acceptance checklist.
- [Instant survey report](design/instant-survey-report.md): reporting rules,
  data-preservation guarantees and local verification limits.
- [Survey workflow improvements](design/survey-improvements.md): reversible
  response preparation, portable recipes, presentation and analysis guidance.

The numbered `project-v1.schema.json` through `project-v6.schema.json` files are
reference schemas used by rejection tests. They describe foreign contracts;
they are not accepted Tessera project formats. The current schema and format
document above define the supported contract.

Earlier development plans and verification records are preserved privately
outside this source repository. Their recorded checks apply to their original
builds and do not establish acceptance of the current source or packages.
