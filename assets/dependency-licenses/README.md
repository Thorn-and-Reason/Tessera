# Supplemental dependency licence texts

Some published Rust crates omit the licence files held in their upstream
repository. These texts fill that packaging gap. `manifest.json` maps exact
crate versions and their `.cargo_vcs_info.json` revisions to the retrieved
files, source URLs and SHA-256 hashes. Where a final line feed was added, the
record retains the original upstream hash and identifies that normalization.
Texts were retrieved on 13 October 2026
from the upstream revision recorded by each crate. Identical text is stored
once. Packaging is offline and fails if a required record, revision or hash
does not match.

For crates that declare MIT but supply only a link or metadata declaration,
the manifest explicitly labels the supplemental standard MIT permission text.
Its source is AccessKit's unmodified MIT text without a copyright header.
This does not attribute the other crate to AccessKit. Package authors remain
listed separately; no new copyright holder or year has been inferred.

The newer objc2 upstream licence explanation is retained, including its
discussion of generated Apple SDK bindings. These records preserve declared
licences and notices; they do not resolve that upstream legal uncertainty or
certify ownership of third-party work.

The packager also includes licence, copyright, NOTICE and AUTHORS files
present in each resolved crate, nested component licence notices, and the
separate embedded-font notices.
For r-efi, AUTHORS contains the MIT grant and copyright attribution. It does
not need a supplemental record.

When dependencies change, review the new crate's actual licence materials
and update this evidence before building a distribution. Do not substitute
a current repository branch for the revision recorded by the crate.
