# Installing Tessera

Use the official package supplied by Thorn & Reason. Keep the licence and
installation documents with your download. Development candidates are labelled
as such; a successful build alone does not establish supported-system acceptance.

Close earlier developer builds before opening this version. The first public
source build uses Tessera's publication-lock namespace; do not save into the
same folder from an earlier developer build at the same time.

## macOS

1. Extract the ZIP and open its `Tessera` folder.
2. Move `Tessera.app` to Applications. The Excel worker is inside the app bundle;
   do not remove it.
3. Open Tessera. Thorn & Reason has chosen to supply Mac packages without
   Developer ID signing or Apple notarization. macOS may block the first launch
   because it cannot verify the developer.
4. If you trust this specific download and macOS offers **Open Anyway**, use
   System Settings → Privacy & Security after the blocked launch, then confirm
   the security prompt. Wording varies by macOS version. Do not disable the
   system's security protections globally.

If macOS reports that the app is damaged, contains malware, or offers no
appropriate approval route, stop and contact Thorn & Reason. Do not remove
quarantine attributes as a routine installation step. Downloaded/quarantined
launch must be tested on each supported macOS version before release.

## Windows

Extract the entire ZIP before opening `tessera.exe`. Keep
`tessera-workbook-worker.exe` beside it; Excel import needs that helper.
The current package is not publisher-signed, so Windows may show an unknown
publisher or reputation warning. Use only a download you trust. If a security
product reports malware, stop and contact Thorn & Reason rather than disabling
it. Native Windows and clean-machine acceptance remain required before release.

## Source and licence

Tessera is GPL-3.0-only. The source is free to obtain, build and redistribute,
including compiled copies, under that licence. The customer downloads page
provides the free corresponding-source link for its packages. See `LICENSE`,
`docs/LICENSING.md` and `THIRD-PARTY-NOTICES.txt` in the package.
