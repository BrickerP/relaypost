# Judge testing instructions

**Draft:** the stable public download is **TBD** until publication is approved.
Use an Apple Silicon Mac. The prototype is ad-hoc signed, without Developer ID
signing or notarization. A Rust toolchain, AI account and application login are
not required to try the prebuilt app.

1. Download the candidate ZIP and adjacent SHA-256 file from **TBD: stable public
   release URL**. Verify the ZIP's checksum, extract it and open `RelayPost.app`.
   If macOS blocks it, review the normal Privacy & Security prompt for this app;
   do not disable system-wide security protections.
2. Click **Try guided demo** on the welcome screen, or **Demo → Run new guided
   demo**. The app creates and shows a **new** output directory. At **Prepared**,
   inspect the parcel and group health, then click **Damage carrier copy →**.
   The demo creates its own isolated identities; no identity setup or password
   from the judge is needed.
3. Inspect the five-shard refusal, then click **Repair and receive →**. Confirm
   four repaired shards and matching SHA-256 values, with the sender still
   **WaitingReceipt**. Click **Carry back signed receipt →** to reach
   **Confirmed**. The generated `demo-report.json` records the same facts;
   **Export JSON report…** copies it to a new chosen filename.

These labels are taken from the candidate source and must be checked against
the final recording before pasting into Devpost. The demo runs two isolated
profiles on one machine using a directory as carrier. It does not test physical
USB media or a second computer. Keep its output if you want to inspect the
evidence; use another new directory to repeat it. Existing output is never
overwritten or resumed.

For command-line verification with the bundled prebuilt executable:

```sh
relaypost demo --output /chosen/new-synthetic-demo --json
```

The command does not open the real default profile and does not require a
password. It runs the same core stages to completion and writes a report. The
longer developer acceptance harness and current validation limits are described
in [DEMO.md](../DEMO.md). Source and remote build instructions are in the
[repository](https://github.com/BrickerP/relaypost).
