# Tessera licensing and distribution

Decision recorded 10 October 2026, following the owner's instruction to choose
the licence best suited to free source code and paid official installers.

## Application licence

The original Tessera application source and build scripts in this repository
are licensed under the **GNU General Public License, version 3 only**
(`GPL-3.0-only`). The product, Cargo package, library and executable are Tessera.
The licence
applies to that code and the original workbook-worker code too.
The complete, unmodified licence is in the repository's `LICENSE` file.

The owner confirmed **Thorn & Reason** as the original application's copyright
holder during production-gate work on 10 October 2026. The About notice uses
that attribution; third-party copyrights and licence notices remain separate.

GPLv3 permits paid distribution and personal, academic and commercial use.
Recipients may build, modify, share or sell copies subject to its terms.
Distributed derivatives must remain GPL-covered and provide corresponding
source in accordance with the licence. Private modifications do not require
public release. The licence does not require users to GPL their ordinary
datasets or analysis results simply because they use Tessera.

This is version 3 only, rather than automatic permission to adopt future GPL
versions. Any future change requires the necessary copyright permissions;
rights already granted for released versions cannot simply be withdrawn.

Third-party source, dependencies and fonts keep their own copyright notices
and licences. This application licence does not relicense those materials.
The supplied Tessera icon and its derived resources are distributed under
GPL-3.0-only; see [artwork provenance](../assets/README.md). GPL permission does not
grant trademark rights or permission to falsely imply that a modified build is
an official Tessera or Thorn & Reason release. This is not a restriction on
redistribution rights granted by the GPL.

## Commercial model

- Make the release's source available free, with practical build instructions.
- Offer official prepared macOS and Windows packages for approximately £10 as
  a one-time purchase, subject to confirmed platform support and final pricing.
- Do not require a subscription, activation server or per-device licence fee.
- Recipients can redistribute installers, including without charge.
- State update and support coverage explicitly at launch; no lifetime promise
  has been made. Purchase terms must preserve GPL rights and applicable
  consumer rights, including rights concerning faulty digital content.

This licensing decision did not itself create a payment account, checkout, sale
or source release. Public source releases now exist for matching published
source snapshots; the website should continue to show development status until
an actual tested package release and checkout are ready.

## Dependency review performed

The locked local Cargo graph was inspected using
`cargo metadata --locked --offline --format-version 1` on 10 October 2026.
All registry packages declared a licence. The declarations provide permissive
options (MIT, Apache-2.0, BSD, ISC, Boost, Zlib or equivalent) for the code;
for example, `self_cell` permits Apache-2.0 as an alternative to GPL-2.0-only.
Select compatible alternatives and preserve the required notices.

`epaint_default_fonts` also declares OFL-1.1 and Ubuntu-font-1.0 for its bundled
fonts. Project font notices are in `assets/fonts/LICENSE` and
`assets/fonts/Libron-LICENSE.txt`; provenance is in that same directory.
Fonts retain their own licences when bundled.
`scripts/package.py` already gathers dependency licence files and embedded
font notices into `THIRD-PARTY-NOTICES.txt`.

This is a review of declared metadata and the existing notices workflow, not
a certification of every dependency's contents, asset provenance or ownership.
Repeat it for the exact final platform builds and check that required notice
texts are actually present in their packages.

## Release requirements

1. Confirm the right to license the original code and distribute included
   assets, including any contributor, employer or other third-party rights.
2. Include `LICENSE`, this notice and third-party notices in every installer
   distribution. The existing package allowlist includes the first two files;
   the package script generates the dependency notices separately.
3. Prepare the complete corresponding source for the exact shipped release,
   including its lockfile, build/packaging scripts, required assets and source
   of non-system dependencies as required by GPLv3. A branch's current tip or
   a Cargo lockfile alone is not a corresponding-source distribution.
4. Provide equivalent access to that source at no further charge, with clear
   directions **next to the installer downloads**, including on the hosted
   delivery page. GPLv3 section 6(d) permits a different source server when
   equivalent copying facilities and clear directions are provided. Maintain
   availability for as long as required. Versioned corresponding-source archives
   and checksums are available on the [public releases page](https://github.com/Thorn-and-Reason/Tessera/releases).
   Each archive identifies a specific source revision; use the archive that
   matches the distributed binaries.
5. Verify a clean build from that source and retain the release identifier,
   source archive and checksums alongside the shipped package identities.
6. Provide an accessible application licence/about notice before the public
   release, including copyright attribution, licence access, redistribution
   rights and the warranty statement qualified by applicable law.
7. Test checkout, receipt, installer delivery, source access and refund handling.
   Confirm all-in pricing and what updates/support the purchase includes.

Existing archives are historical development packages; adding these files does
not rewrite them or make them ready for commercial publication.

## References

- Full licence: https://www.gnu.org/licenses/gpl-3.0.html
- Charging for copies: https://www.gnu.org/licenses/gpl-faq.html#DoesTheGPLAllowMoney
- Source alongside downloads: GPLv3 section 6(d), in `LICENSE`.
- Applying the licence: https://www.gnu.org/licenses/gpl-howto.html
