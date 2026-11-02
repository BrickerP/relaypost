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
import sqlite3
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


def validate_guided_command(binary, root, transcript):
    """Independently inspect the shared engine's output, never implement its flow."""
    output = root / "guided-core"
    blocked_home = root / "SYNTHETIC-not-a-profile"
    blocked_home.write_bytes(b"SYNTHETIC must remain a regular file")
    command = [str(binary), "--home", str(blocked_home),
               "--password-file", str(root / "does-not-exist.password"),
               "demo", "--output", str(output), "--json"]
    print("> shared guided demo (profile and password input must be ignored)", flush=True)
    result = subprocess.run(command, input="", capture_output=True, text=True,
                            check=False, timeout=300)
    transcript.append({"profile": "guided-core (isolated)",
                       "arguments": ["demo", "--output", str(output), "--json"],
                       "exit_code": result.returncode})
    check(result.returncode == 0, f"Guided demo failed: {result.stderr.strip()}")
    report = json.loads(result.stdout)
    check(report["phase"] == "Confirmed", "Guided command did not finish all real stages")
    check(report["sender_state"] == "Confirmed", "Guided sender is not confirmed")
    check(report["root"] == str(output), "Guided demo escaped its requested root")
    report_path = Path(report["report_path"])
    check(report_path == output / "demo-report.json", "Unexpected guided report path")
    check(json.loads(report_path.read_text(encoding="utf-8")) == report,
          "Guided stdout must match the immutable report")

    def hashes(directory):
        return {path.relative_to(directory).as_posix(): digest(path)
                for path in sorted(directory.rglob("*")) if path.is_file()}

    sources = output / "source_files"
    check(sum(path.stat().st_size for path in sources.rglob("*") if path.is_file())
          == 21 * MIB, "Guided sample is not 21 MiB")
    check(hashes(sources) == report["source_sha256"], "Guided source digests differ from disk")
    received_path = Path(report["received_path"])
    check(received_path.is_relative_to(output), "Guided received files escaped their root")
    check(hashes(received_path) == report["received_sha256"] == report["source_sha256"],
          "Guided received bytes do not match both reported and source digests")
    check(report["repaired_shards"] == 4, "Guided repair did not reconstruct four shards")

    parcel = Path(report["carrier_parcel_path"])
    envelope_bytes = (parcel / "envelope.json").read_bytes()
    envelope = json.loads(envelope_bytes)
    group = envelope["groups"][0]
    actual_bad = []
    for index, shard in enumerate(group["shards"]):
        path = parcel / shard["path"]
        if not path.is_file() or path.stat().st_size != group["shard_bytes"] \
                or digest(path) != shard["sha256"]:
            actual_bad.append(index)
    check(len(actual_bad) == 4 and sum(index < 20 for index in actual_bad) == 2,
          "Guided carrier does not contain two data and two parity erasures")
    check(actual_bad == report["damaged_shard_indices"]
          == report["inspection_after_damage"]["groups"][0]["missing_shards"],
          "Guided inspection does not match real shard bytes")
    refusal = report["five_erasure_refusal_evidence"]
    check(not refusal["recoverable"] and len(refusal["groups"][0]["missing_shards"]) == 5
          and bool(report["five_erasure_receive_error"]),
          "Guided five-erasure case lacks real receive-refusal evidence")

    # Reassemble pristine data shards, without invoking the demo's codec.
    pristine = output / "alice/outbox" / envelope["parcel_id"]
    ciphertext_hash = hashlib.sha256()
    ciphertext_size = 0
    for item in envelope["groups"]:
        data = b"".join((pristine / shard["path"]).read_bytes()
                        for shard in item["shards"][:item["data_shards"]])[:item["bytes"]]
        ciphertext_hash.update(data)
        ciphertext_size += len(data)
    check(ciphertext_hash.hexdigest() == envelope["parcel_id"]
          and ciphertext_size == envelope["ciphertext_bytes"],
          "Guided parcel ID is not the actual retained ciphertext hash")
    receipt = json.loads((Path(report["receipt_path"]) / "receipt.json").read_text())
    check(receipt["parcel_id"] == envelope["parcel_id"]
          and receipt["envelope_sha256"] == hashlib.sha256(envelope_bytes).hexdigest()
          and receipt["result"] == "committed", "Guided receipt does not bind the real parcel")
    connection = sqlite3.connect(output / "alice/state.sqlite3")
    try:
        state = connection.execute(
            "SELECT record_json FROM deliveries WHERE direction = 'Outgoing' AND parcel_id = ?",
            (envelope["parcel_id"],)).fetchone()
    finally:
        connection.close()
    check(state and json.loads(state[0])["state"] == "Confirmed",
          "Guided report differs from the real sender index")
    check(blocked_home.read_bytes() == b"SYNTHETIC must remain a regular file",
          "Demo command touched the requested normal profile")
    for forbidden in ("AGE-SECRET-KEY-", "synthetic-demo-alice-v1", "synthetic-demo-bob-v1"):
        check(forbidden not in result.stdout, "Guided report exposed synthetic secret material")
    report_hash = digest(report_path)
    repeated = subprocess.run(command, input="", capture_output=True, text=True,
                              check=False, timeout=30)
    check(repeated.returncode != 0 and digest(report_path) == report_hash,
          "Guided command must refuse an existing root without overwriting its report")
    return {"report": str(report_path), "phase": report["phase"],
            "independent_disk_validation": True, "normal_profile_ignored": True,
            "password_input_ignored": True, "existing_root_refused": True}


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

    def cli(profile, *arguments, expect_error=False, with_password=True):
        display = f"{profile}: {' '.join(str(arg) for arg in arguments)}"
        print(f"> {display}", flush=True)
        command = [str(binary), "--home", str(root / profile), "--json"]
        if with_password:
            command.extend(["--password-file", str(password)])
        command.extend(map(str, arguments))
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
    cli("alice", "contacts", "import", alice_card, expect_error=True)
    cli("alice", "seal", "--to", alice["fingerprint"], source, expect_error=True)
    check(cli("alice", "deliveries") == [], "Self-send must not create a delivery")
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

    # Recreate the private durable prepublication state from authenticated
    # synthetic output. This is a crash-window fixture, not a power-loss test.
    prepared_stage = root / "bob/inbox/.staging/batch-SYNTHETIC-demo-ready"
    prepared_stage.mkdir(mode=0o700)
    shutil.move(str(destination.parent), prepared_stage / "batch")
    (prepared_stage / "archive.tar").write_bytes(b"SYNTHETIC retained plaintext archive")
    (prepared_stage / "ciphertext.age").write_bytes(b"SYNTHETIC retained ciphertext")
    connection = sqlite3.connect(root / "bob/state.sqlite3")
    try:
        connection.execute("DELETE FROM deliveries WHERE direction = 'Incoming' AND parcel_id = ?",
                           (parcel_id,))
        connection.commit()
    finally:
        connection.close()
    shutil.rmtree(Path(received["record"]["receipt_path"]))
    resumed = cli("bob", "recover")
    check(resumed["recovered"] == 1 and resumed["pending"] == 0 and not resumed["conflicts"],
          "Recovery must publish a verified prepared stage and regenerate its receipt")
    check(not prepared_stage.exists(), "Recovery must remove retained plaintext staging data")
    check(digest(matches[0]) == source_hash, "Resumed publication changed authenticated payload")
    cli("bob", "receipt", "export", parcel_id, "--carrier", root / "resumed-return")

    # Publication renames the batch child out of its wrapper. Exercise the real
    # residue shape too: only the ID-bound wrapper's archive/ciphertext remain.
    postpublication_stage = root / f"bob/inbox/.staging/batch-{parcel_id}-demo-committed"
    postpublication_stage.mkdir(mode=0o700)
    (postpublication_stage / "archive.tar").write_bytes(b"SYNTHETIC postpublication plaintext")
    (postpublication_stage / "ciphertext.age").write_bytes(b"SYNTHETIC postpublication ciphertext")
    cleaned = cli("bob", "recover")
    check(cleaned["pending"] == 0 and not cleaned["conflicts"] and not postpublication_stage.exists(),
          "Recovery must verify the final batch before deleting its postpublication residue")
    check(digest(matches[0]) == source_hash, "Postpublication cleanup changed the payload")

    partial_stage = root / "bob/inbox/.staging/batch-SYNTHETIC-demo-partial"
    partial_stage.mkdir(mode=0o700)
    (partial_stage / "archive.tar").write_bytes(b"SYNTHETIC incomplete receive plaintext")
    pending = cli("bob", "recover")
    check(pending["pending"] == 1 and partial_stage.exists(),
          "Normal recovery must retain incomplete staging data for an explicit decision")
    discarded = cli("bob", "recover", "--discard-incomplete", with_password=False)
    check(discarded["discarded_staging"] == 1 and not partial_stage.exists(),
          "Explicit cancellation must remove the managed incomplete stage without unlocking")
    check(digest(matches[0]) == source_hash, "Staging cancellation changed the published payload")
    check(cli("bob", "recover")["pending"] == 0, "Cancelled staging must no longer be pending")

    source_shard = Path(record["parcel_path"]) / "groups/g000/s000.bin"
    retained_bytes = source_shard.read_bytes()
    damaged_bytes = bytearray(retained_bytes)
    damaged_bytes[0] ^= 0x80
    source_shard.write_bytes(damaged_bytes)
    cli("alice", "recover")
    unhealthy = next(row for row in cli("alice", "deliveries") if row["parcel_id"] == parcel_id)
    check(unhealthy["state"] == "Confirmed" and unhealthy.get("local_error"),
          "Local shard damage must be reported without erasing recipient confirmation")
    source_shard.write_bytes(retained_bytes)
    cli("alice", "recover")
    restored = next(row for row in cli("alice", "deliveries") if row["parcel_id"] == parcel_id)
    check(restored["state"] == "Confirmed" and not restored.get("local_error"),
          "Exact outgoing restoration must clear local health without changing delivery phase")
    check(digest(source) == source_hash, "Source file was changed")
    guided = validate_guided_command(binary, root, transcript)
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
        "self_send_rejected": True,
        "prepared_stage_resumed": True,
        "postpublication_stage_cleaned": True,
        "incomplete_stage_explicitly_discarded": True,
        "confirmed_health_restored": True,
        "shared_guided_demo": guided,
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
