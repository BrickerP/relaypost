#!/usr/bin/env python3
"""Exercise a prebuilt CLI with synthetic data and isolated local profiles.

Never compiles Rust, reads a real identity, or contacts a network. This is an
isolated-process demonstration on one machine, not a two-device test.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

MIB = 1024 * 1024


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(MIB), b""):
            result.update(block)
    return result.hexdigest()


def check(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path,
                        help="Path to the prebuilt relaypost CLI")
    parser.add_argument("--output", required=True, type=Path,
                        help="New, empty directory for synthetic fixtures and report")
    args = parser.parse_args()
    binary = args.binary.resolve()
    check(binary.is_file(), f"Prebuilt CLI not found: {binary}")
    root = args.output.resolve()
    check(not root.exists() or not any(root.iterdir()),
          f"Refusing to reuse a nonempty demo directory: {root}")
    root.mkdir(parents=True, exist_ok=True)
    password = root / "SYNTHETIC-TEST-ONLY.password"
    password.write_text("SYNTHETIC-DEMO-ONLY-not-a-user-secret\n", encoding="utf-8")
    password.chmod(0o600)
    transcript = []

    def cli(profile, *arguments, expect_error=False):
        display = f"{profile}: {' '.join(str(arg) for arg in arguments)}"
        print(f"> {display}", flush=True)
        command = [str(binary), "--home", str(root / profile), "--json",
                   "--password-file", str(password), *map(str, arguments)]
        result = subprocess.run(command, capture_output=True, text=True, check=False)
        transcript.append({"profile": profile, "arguments": list(map(str, arguments)),
                           "exit_code": result.returncode})
        if expect_error:
            check(result.returncode != 0, f"Command unexpectedly succeeded: {display}")
            return None
        check(result.returncode == 0,
              f"Command failed: {display}\n{result.stderr.strip()}")
        try:
            return json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise RuntimeError(f"CLI did not return JSON: {display}\n{result.stdout}") from error

    alice = cli("alice", "identity", "init", "--name", "Synthetic Alice")
    bob = cli("bob", "identity", "init", "--name", "Synthetic Bob")
    alice_card, bob_card = root / "alice.card", root / "bob.card"
    cli("alice", "identity", "export", "--output", alice_card)
    cli("bob", "identity", "export", "--output", bob_card)
    cli("alice", "contacts", "import", bob_card)
    cli("bob", "contacts", "import", alice_card)
    cli("alice", "contacts", "trust", bob["fingerprint"])
    cli("bob", "contacts", "trust", alice["fingerprint"])

    source = root / "synthetic-payload.bin"
    # A 21 MiB static regular file gives a full 20+4 group and a tail.
    pattern = bytes(range(251))
    with source.open("wb") as stream:
        remaining = 21 * MIB
        while remaining:
            block = (pattern * (MIB // len(pattern) + 1))[:min(MIB, remaining)]
            stream.write(block)
            remaining -= len(block)
    source_hash = digest(source)
    record = cli("alice", "seal", "--to", bob["fingerprint"], source)
    parcel_id = record["parcel_id"]
    pristine = Path(cli("alice", "export", parcel_id, "--carrier", root / "carrier"))
    waiting = cli("alice", "deliveries")
    check(next(row for row in waiting if row["parcel_id"] == parcel_id)["state"]
          == "WaitingReceipt", "Export must wait for a recipient receipt")
    initial = cli("bob", "inspect", pristine)
    check(initial["groups"][0]["data_shards"] == 20, "Expected full first group")
    check(initial["groups"][0]["parity_shards"] == 4, "Expected four parity shards")

    excessive = root / "five-bad.parcel"
    shutil.copytree(pristine, excessive)
    for index in (0, 1, 2, 20, 23):
        (excessive / f"groups/g000/s{index:03}.bin").unlink()
    failed_inspection = cli("bob", "inspect", excessive)
    check(not failed_inspection["recoverable"], "Five erasures must exceed the first-group budget")
    cli("bob", "receive", excessive, expect_error=True)
    cli("bob", "receipt", "export", parcel_id, "--carrier", root / "forbidden-return",
        expect_error=True)
    check(not any(row["state"] == "SavedReceiptReady" for row in cli("bob", "deliveries")),
          "Over-budget input must not publish a successful delivery")

    damaged = root / "four-bad.parcel"
    shutil.copytree(pristine, damaged)
    for index in (0, 1):
        (damaged / f"groups/g000/s{index:03}.bin").unlink()
    for index in (20, 23):
        path = damaged / f"groups/g000/s{index:03}.bin"
        data = bytearray(path.read_bytes())
        data[len(data) // 2] ^= 0x80
        path.write_bytes(data)
    damaged_inspection = cli("bob", "inspect", damaged)
    check(damaged_inspection["recoverable"], "Four erasures should be recoverable")
    check(len(damaged_inspection["groups"][0]["missing_shards"]) == 4,
          "Missing and hash-bad parity shards must both be reported as erasures")
    received = cli("bob", "receive", damaged)
    check(received["repaired_shards"] == 4 and not received["duplicate"],
          "Expected four repaired shards on the first import")
    destination = Path(received["record"]["received_path"])
    matches = list(destination.rglob(source.name))
    check(len(matches) == 1, "Expected exactly one committed synthetic payload")
    received_hash = digest(matches[0])
    check(received_hash == source_hash, "Recovered bytes differ from source")
    duplicate = cli("bob", "receive", damaged)
    check(duplicate["duplicate"], "Second import must be idempotent")
    check(duplicate["record"]["received_path"] == str(destination),
          "Duplicate import must retain the same destination")
    check(len([row for row in cli("bob", "deliveries")
               if row["parcel_id"] == parcel_id]) == 1, "Duplicate delivery row")
    check(next(row for row in cli("alice", "deliveries")
               if row["parcel_id"] == parcel_id)["state"] == "WaitingReceipt",
          "Sender must still wait until the receipt returns")
    receipt = Path(cli("bob", "receipt", "export", parcel_id,
                       "--carrier", root / "return-carrier"))
    confirmed = cli("alice", "receipt", "import", receipt)
    check(confirmed["state"] == "Confirmed", "Valid return receipt must confirm delivery")
    check(digest(source) == source_hash, "Source file was changed")
    report = {
        "conditions": "two isolated profiles and separate CLI processes on one machine; directory carrier",
        "synthetic_data_only": True,
        "parcel_id": parcel_id,
        "source_sha256": source_hash,
        "received_sha256": received_hash,
        "initial_groups": initial["groups"],
        "damaged_groups": damaged_inspection["groups"],
        "repaired_shards": received["repaired_shards"],
        "duplicate": duplicate["duplicate"],
        "final_sender_state": confirmed["state"],
        "over_budget_rejected": True,
        "commands": transcript,
    }
    (root / "demo-report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: value for key, value in report.items() if key != "commands"}, indent=2))
    print(f"Synthetic demo passed. Report: {root / 'demo-report.json'}")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError) as error:
        print(f"Demo failed: {error}", file=sys.stderr)
        sys.exit(1)
