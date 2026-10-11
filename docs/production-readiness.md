# Production-readiness requirements

Tessera 0.3.0 is a local development candidate. Automated success and an ad-hoc
signature do not authorise public production distribution.

## Required release gates

| Gate | Acceptance condition | Owner/environment |
| --- | --- | --- |
| Source freeze | Reviewed source snapshot and build-input digest identify the shipped binaries; all intended changes included | Maintainer |
| Automated verification | Complete application/worker suites, schema/package checks, format and coverage ≥80% pass for the candidate | Build machine / CI |
| Dependency review | Current vulnerability audit; explicitly review maintenance advisories and required notices | Maintainer |
| Mac native acceptance | Test modes, import, all procedure families, figures/reports, Save/Open, Undo/Redo, cancellation, dirty Quit and deliberate crash/recovery using disposable data | Accessible macOS session |
| Accessibility | Keyboard-only and actual screen-reader walkthrough at supported window sizes/scales | Native tester |
| Clean install | Extract/install on a separate user or clean machine, launch both architectures on the claimed OS baseline, verify Excel helper, permissions and recovery storage | Mac ARM64/Intel testers |
| Mac distribution | Owner chose unsigned/ad-hoc packages: include clear installation/security-prompt instructions and verify downloaded/quarantined launch on supported systems; do not claim Developer ID signing or notarization | Mac testers / maintainer |
| Windows release | Native GUI, filesystem/DACL, worker and clean-machine acceptance; recovery remains unavailable until directory durability is verified | Windows tester |
| Linux release | Non-root complete suite for each declared OS/architecture and GUI/portal/publication/recovery acceptance | Linux tester |
| Source/licence delivery | Full licence, notices and exact corresponding source with build instructions delivered alongside the installer, at no additional charge | Maintainer / publisher |
| Publication | Final version/build identity, release notes, checksum, source location and support/update terms; checkout/delivery tested if sold | Owner |

Windows and Linux must remain labelled unverified/unreleased until their own
gates pass. AppImage and portable user-data storage are not implemented claims.
Manual edit summaries are not replayable preparation payloads.

## Mac native acceptance record

Record tester, OS/architecture, app/package/source SHA-256, fresh-profile state,
observed values, output paths and pass/fail for each scenario. Use copied synthetic
fixtures; never overwrite a user's working project.

1. Start Guided, switch to Advanced, quit normally and relaunch; confirm preference.
2. Open each bundled example; switch modes and verify unchanged saved values.
3. Import UTF-8 CSV/TSV and a two-sheet `.xlsx`; cancel an import and retain prior data.
4. Exercise a valid and invalid configuration for each of the twelve procedures.
5. Save a project, reopen in a fresh process, retain stale results after editing,
   and export HTML plus standalone SVG/PNG; inspect exact tables and labels.
6. Compute/recode/sort/filter; Undo/Redo and branch edits; verify revision/identity safety.
7. Cancel a running job and test dirty Quit → Cancel and dirty Quit → Save.
8. Edit disposable data, wait beyond the checkpoint debounce, force-terminate
   only the test process, relaunch and explicitly recover. Verify exact unsaved
   content and that the original saved file was not overwritten.
9. Repeat principal controls with keyboard and VoiceOver; check light/dark,
   640×440, 1180×780 and 150% scaling.

On 11 October, native desktop control verified mode persistence, CSV import,
dirty Quit cancellation, project saving and deliberate crash/recovery with
disposable data. This closes those observed scenarios for the tested build;
the remaining all-procedure, keyboard-only and screen-reader walkthroughs,
clean-machine and platform gates still require acceptance records.
