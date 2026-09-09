# Releasing mbr

Pushing a `vX.Y.Z` tag builds, signs, notarizes and publishes mbr's macOS DMG
(and the Linux/Windows tarballs, and the crate). `.github/workflows/release.yml`
is the whole pipeline; this document is the part that cannot live in a comment
— the one-time Apple setup, and what to do when notarization says no.

## What ships, and where signing happens

One DMG, `mbr-macos-arm64.dmg` (mbr is Apple Silicon-only — see the `build`
job's comment on why the Intel leg was dropped), built by `scripts/make-macos-
dmg.sh` in the `package-dmg` job and attached to the GitHub Release alongside
the Linux/Windows tarballs and the raw `mbr-macos-arm64.tar.gz`/`mbr-cli-macos-
arm64.tar.gz` archives.

**Only the DMG is really signed.** `MBR.app` is ad-hoc signed inside the Nix
sandbox at build time (`flake.nix`, both the `mbr-quicklook`/`mbr` derivations
and the `release` derivation) — that signing is hermetic and secret-free by
design, and stays that way; see `docs/reference/development.md`'s "Code
signing: never strip an ad-hoc signature" section for why it must never be
removed. `scripts/make-macos-dmg.sh` re-signs the bundle on the CI runner
(outside the Nix sandbox, where `codesign` can actually reach its daemon)
immediately before wrapping it in the DMG, and *that* re-sign is where a real
Developer ID identity, hardened runtime, notarization and stapling happen —
for both the `.app` and the `.dmg`. The `.tar.gz` app bundle is never touched
by that script and always ships ad-hoc signed only; a user who prefers the
tarball over the DMG still needs the `xattr -dr com.apple.quarantine` /
right-click-Open workaround.

### Why the .app is signed and notarized before the DMG is built

Stapling the DMG alone (the common shortcut) leaves the app itself ticketless,
so a user who drags it to `/Applications` and launches it offline gets a
Gatekeeper failure the first time. Two notarization round trips — the app,
then the DMG — is the price of the app working without a network.

### Signing order is innermost-out

`scripts/make-macos-dmg.sh` signs `Contents/Frameworks/libpdfium.dylib`, then
`Contents/PlugIns/MBRPreview.appex` (with its entitlements —
`quicklook/MBRPreview/MBRPreview.entitlements`), then `MBR.app` itself. A
signature over the bundle seals its contents, so signing a nested binary
afterwards would invalidate the outer one. `--deep` is deprecated for signing
on macOS 13+ (it silently applies the same options to nested code and misses
anything it does not recognise) so each Mach-O is signed explicitly; a future
addition to the bundle has to be added to that script by name, which is the
right failure mode — an unsigned nested binary fails notarization with a
report that names it, rather than shipping quietly unsigned.

### Ad-hoc fallback, and the `dry_run` input

`scripts/make-macos-dmg.sh` signs for real only when both `IDENTITY` and
`KEYCHAIN` are set in its environment. Empty (the default) reproduces the
script's original, secret-free behavior exactly: ad-hoc signing only, no
notarization, no network calls — a DMG that opens with Gatekeeper's
recoverable "Open Anyway" flow rather than notarized-and-silent.

The `release` workflow's `workflow_dispatch` trigger has a `dry_run` boolean
input, **default `true`**, mirroring the same input on `ledgeline`'s release
workflow (the sibling project this pipeline was ported from). With `dry_run`
true — or omitted — the whole pipeline builds and packages exactly as a real
release does, using the ad-hoc fallback above, and stops before the GitHub
Release and crates.io publish steps (`validate-version`'s `sign` output gates
`package-dmg`'s signing steps and the `release`/`publish-crate` jobs; see the
comments on that output and on those jobs). Setting `dry_run: false` on a
manual dispatch signs for real, exactly like a tag push does — useful for
re-issuing a release without cutting a new tag, or for a full signing dry run
before trusting a real one.

Use a `dry_run` dispatch whenever the packaging changes. It is the only way to
exercise the DMG layout, and the ad-hoc-signing fallback path, without holding
the signing secrets or spending a tag.

## Cutting a release

```sh
./scripts/bump-version.sh 0.7.0   # rewrites Cargo.toml, updates Cargo.lock, runs benchmarks
git diff                          # review
git add -A && git commit -m "Release v0.7.0"
git tag v0.7.0
git push origin main v0.7.0       # pushing the tag starts the release
```

`validate-version` (the workflow's first real job) fails the run if the tag
does not match `Cargo.toml`'s `version`, so a mistagged push fails loudly there
rather than shipping a DMG whose About panel disagrees with its own filename.

## One-time Apple setup

Five repository secrets, added under **Settings → Secrets and variables →
Actions** on `zmre/mbr-markdown-browser`. You need an Apple Developer Program
membership ($99/yr) — reuse the same Developer ID certificate and App Store
Connect API key already provisioned for `ledgeline`; nothing here requires a
second enrollment.

### The signing certificate

`MACOS_CERTIFICATE_P12`, `MACOS_CERTIFICATE_PASSWORD`

You want a **Developer ID Application** certificate — *not* "Mac App
Distribution", which only works for the App Store and will fail notarization
for a directly distributed app.

1. In Xcode: **Settings → Accounts → your team → Manage Certificates → + →
   Developer ID Application**. (Or create the CSR by hand at
   <https://developer.apple.com/account/resources/certificates>.)
2. In **Keychain Access**, find the certificate, expand it so both the cert and
   its private key are selected, right-click → **Export 2 items…**, save as
   `.p12`, and set a strong password. Both halves matter: a `.p12` exported
   without the private key imports fine and then fails at `codesign` with "no
   identity found".
3. Base64 it, and put the result in `MACOS_CERTIFICATE_P12`:

   ```sh
   base64 -i DeveloperID.p12 | pbcopy
   ```

4. Put the export password in `MACOS_CERTIFICATE_PASSWORD`.

The workflow derives the signing identity string from the certificate itself
(`security find-identity -v -p codesigning`), so there is no sixth secret to
keep in sync — a hand-typed "Developer ID Application: ..." string would drift
from the actual cert and fail with an unhelpful "no identity found".

### The notarization key

`APPLE_API_KEY_P8`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID`

An App Store Connect API key, rather than an Apple ID and app-specific
password: it does not break when 2FA prompts, it is scoped, and it is
revocable on its own.

1. At <https://appstoreconnect.apple.com/access/integrations/api>, create a key
   with the **Developer** role (that is sufficient for notarization).
2. Download `AuthKey_XXXXXXXXXX.p8`. **Apple lets you download it exactly
   once.**
3. `APPLE_API_KEY_P8` — `base64 -i AuthKey_XXXXXXXXXX.p8 | pbcopy`
4. `APPLE_API_KEY_ID` — the ten-character Key ID (the `XXXXXXXXXX` above).
5. `APPLE_API_ISSUER_ID` — the UUID shown above the key list on that page. It
   is per-team, not per-key.

## Verifying by hand

`scripts/make-macos-dmg.sh` asserts all of this itself before finishing (when
signing for real), but when something looks wrong on a downloaded DMG:

```sh
# The ticket is stapled and valid offline.
xcrun stapler validate mbr-macos-arm64.dmg

# The string that matters is `source=Notarized Developer ID`. Anything else
# means a user gets a Gatekeeper prompt.
spctl --assess --type exec -vvv /Applications/MBR.app

# Every load command resolves on a stock Mac -- no /nix/store.
otool -L /Applications/MBR.app/Contents/MacOS/mbr
```

## When notarization fails

`scripts/notarize.sh` (called once for the `.app`, once for the `.dmg`) prints
the full notary log and fails loudly on anything but an `Accepted` verdict.
Common causes:

| Symptom in the log | Cause | Fix |
| --- | --- | --- |
| `The signature does not include a secure timestamp` | `--timestamp` missing, or Apple's timestamp server was unreachable | Re-run; it is usually transient |
| `The executable does not have the hardened runtime enabled` | A Mach-O got signed without `--options runtime` | Add it to `sign()` in `scripts/make-macos-dmg.sh` |
| `The binary is not signed` naming a path under `Contents/` | A new nested binary was added to the bundle but not to the sign step | Add it to `scripts/make-macos-dmg.sh` explicitly, on purpose (see "Signing order" above) |
| `Team is not yet configured for notarization` | New membership, agreements not accepted | Accept the current agreements in App Store Connect |

**Entitlements.** The main app needs none: it is not sandboxed, and `wry`
drives `WKWebView`, whose JIT lives in the out-of-process
`com.apple.WebKit.WebContent` system service rather than in mbr's own address
space, so `com.apple.security.cs.allow-jit` and its relatives are not needed.
`MBRPreview.appex` (the QuickLook extension) does carry an entitlement — a
broad, deliberately-scoped-in-code temporary-exception file-read grant; see the
comment in `quicklook/MBRPreview/MBRPreview.entitlements` for why it is that
wide and what actually constrains it. If notarization ever rejects either for
entitlement reasons, that is the first thing to revisit.

## Publishing to crates.io

`publish-crate` runs after the GitHub Release, gated (via `needs: release`) the
same way the release itself is — a dry run publishes nothing. It uses
`CARGO_REGISTRY_TOKEN`, which predates this pipeline and is unrelated to the
five Apple secrets above.
