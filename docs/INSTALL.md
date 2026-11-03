# Install RelayPost 0.3.0

The release packages include prebuilt programs. Rust, Cargo, Python and a source
checkout are not needed to run them. Use the assets attached to the versioned
[GitHub Release](https://github.com/BrickerP/relaypost/releases) when it is published.
Until then, the successful **Verify and package** run's
`relaypost-0.3.0-candidate` artifact is a review candidate, not a public release.
Downloading Actions artifacts requires a GitHub account; public Release assets do not.

| Package | Scope and tested baseline |
| --- | --- |
| `relaypost-0.3.0-macos-arm64.zip` | Apple Silicon native app and bundled CLI; built with a macOS 12.0 deployment target, exercised on macOS 15. macOS 12 itself has not been exercised. Ad-hoc signed, without Developer ID signing or notarization. |
| `relaypost-0.3.0-linux-x86_64.tar.gz` | CLI only; Ubuntu 24.04 x86_64 / glibc 2.39 baseline. Other distributions have not been verified. |

Intel Macs, Linux ARM, Linux desktop UI and Windows packages are not available.

## macOS: start without Terminal

1. Download the Apple Silicon ZIP from the matching release or checked review
   candidate. Double-click it in Finder to extract it.
2. Drag `relaypost-0.3.0-macos-arm64/RelayPost.app` into **Applications** and open
   it. This preview is ad-hoc signed, without Developer ID signing or Apple
   notarization. If macOS asks you to approve the expected app, review it in
   **System Settings → Privacy & Security** after the opening attempt. Managed
   Macs may disallow approval. Do not disable Gatekeeper or remove quarantine
   attributes.
3. Start on **Send files**. Set up this computer, then use **Add recipient**
   to find the nearby computer and compare every group of
   the complete safety code on both computers. To try a disposable sample first,
   open **Help & walkthrough → Start a new walkthrough**; no identity setup or
   password is needed for that sample.

Open RelayPost on both computers on the same local network. Accept the pairing
request and check the complete safety code on each side. Being saved as a
recipient does not mean a computer is currently available. In **Send files**,
select an available verified recipient and choose **Send directly**. The other
person accepts the request before the encrypted package is transferred, then
explicitly unlocks and saves in **Receive files**. A signed confirmation returns
automatically while both apps are available; **Transfers** shows progress.

For disconnected use, add the other computer using its public identity card
and choose **Save transfer package**. Move it using a USB drive, shared folder
or another accessible location, and open it in the recipient's **Receive files**.
Return its confirmation package and import it in **Transfers**. A package
addressed to someone else is never incoming for this computer. Both delivery
methods confirm only after the expected computer's signed save receipt verifies.

The normal desktop workflow requires no command-line tools. Files are saved in
the app's inbox; the result offers **Show saved files**. There is no online
account or cloud service.

### Optional: verify the downloaded package

Download the adjacent `.zip.sha256` file from the same release. In Terminal,
change to that download directory and run:

```sh
shasum -a 256 --check relaypost-0.3.0-macos-arm64.zip.sha256
ditto -x -k relaypost-0.3.0-macos-arm64.zip .
bash relaypost-0.3.0-macos-arm64/verify-package.sh relaypost-0.3.0-macos-arm64
```

The final command verifies packaged files, both executables, the bundle signature
and version. It prints the full source commit; compare it with `RELEASE.json` and
the release notes. Pass the expected 40-character commit as a second argument
for an exact match. A checksum detects changed bytes; it does not independently
identify the author.

The bundled CLI is at:

```sh
/Applications/RelayPost.app/Contents/MacOS/relaypost --version
/Applications/RelayPost.app/Contents/MacOS/relaypost demo --output "$HOME/RelayPost-demo-new"
```

Choose a new demo output directory for each run. Its fixture passphrase is only
for the synthetic demo, not for real identities. The demo does not inspect your
normal profile. See the bundled `DEMO.md` for what it proves and its limits.

## Linux CLI: three steps

1. Download the `.tar.gz` and adjacent `.tar.gz.sha256` from the same release and
   verify before extraction:

   ```sh
   sha256sum --check relaypost-0.3.0-linux-x86_64.tar.gz.sha256
   tar -xzf relaypost-0.3.0-linux-x86_64.tar.gz
   bash relaypost-0.3.0-linux-x86_64/verify-package.sh relaypost-0.3.0-linux-x86_64
   ```

2. Run the CLI in place; installation and `sudo` are unnecessary:

   ```sh
   ./relaypost-0.3.0-linux-x86_64/bin/relaypost --help
   ```

3. Run the real synthetic exchange in a new directory:

   ```sh
   ./relaypost-0.3.0-linux-x86_64/bin/relaypost demo --output "$HOME/RelayPost-demo-new"
   ```

The demo uses local directories as carriers on one machine. It does not establish
two-device interoperability, real USB behavior, or physical power-loss durability.

## Profiles, licenses and removal

`--home DIRECTORY` selects an isolated profile; otherwise the CLI and UI share
the OS application-data profile. Never put passwords in command arguments. Use
a masked prompt, `--password-stdin`, or `--password-file` for real operations.

The archive carries this project's MIT `LICENSE`, `THIRD_PARTY_NOTICES.md`, a
machine-readable `DEPENDENCIES.json`, and the dependencies' supplied license and
notice files under `licenses/`. The macOS package also includes its bundled-font
inventory, original OFL and pinned font source record. The macOS app keeps these in
`Contents/Resources`, so moving the app preserves its notices. Dependencies have
their own license terms; consult the pinned upstream source for inventory entries
when redistributing or modifying them. Workspace license texts omitted from
published crates are included from the checked-in source catalog. Original
copyright notices, source commits or crate checksums, URLs and text hashes are
retained in the inventory. Entries using a primary canonical license text retain
the exact upstream declaration and real author metadata; they do not invent a
copyright holder or year.

To remove the app, delete `RelayPost.app` (or the extracted Linux directory).
Profile data and demo output are separate: uninstalling does not remove your
identity or received files. Back up protected real identity profiles before
deliberately deleting them; there is no password reset or key-recovery service.
