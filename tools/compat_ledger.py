#!/usr/bin/env python3
"""Report evidence-weighted Java-mod compatibility from the tracked ledger."""

import argparse
import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LEDGER = ROOT / "compat-java" / "compat-ledger.json"
STATUSES = {"supported", "partial", "unsupported"}


def load(path=DEFAULT_LEDGER):
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    validate(data)
    return data


def validate(data):
    if data.get("schema") != 1:
        raise ValueError(f"unsupported compat ledger schema {data.get('schema')!r}")
    area_ids = set()
    member_ids = set()
    for area in data.get("areas", []):
        area_id = area.get("id")
        if not area_id or area_id in area_ids:
            raise ValueError(f"invalid or duplicate area id {area_id!r}")
        area_ids.add(area_id)
        if not area.get("members"):
            raise ValueError(f"area {area_id} has no members")
        for member in area["members"]:
            member_id = f"{area_id}.{member.get('id', '')}"
            if member_id in member_ids or not member.get("id"):
                raise ValueError(f"invalid or duplicate member id {member_id!r}")
            member_ids.add(member_id)
            if member.get("status") not in STATUSES:
                raise ValueError(f"{member_id} has invalid status {member.get('status')!r}")
            weight = member.get("weight")
            if not isinstance(weight, int) or weight <= 0:
                raise ValueError(f"{member_id} has invalid weight {weight!r}")
            if member["status"] == "supported" and not member.get("evidence"):
                raise ValueError(f"{member_id} is supported without test evidence")


def area_status(area):
    statuses = {member["status"] for member in area["members"]}
    if statuses == {"supported"}:
        return "supported"
    if "supported" in statuses or "partial" in statuses:
        return "partial"
    return "unsupported"


def summary(data):
    supported = sum(
        member["weight"]
        for area in data["areas"]
        for member in area["members"]
        if member["status"] == "supported"
    )
    total = sum(member["weight"] for area in data["areas"] for member in area["members"])
    return {
        "minecraft": data["minecraft"],
        "loader": data["loader"],
        "supported_weight": supported,
        "total_weight": total,
        "percentage": round(100 * supported / total, 1) if total else 0.0,
        "areas": [
            {
                "id": area["id"],
                "phase": area["phase"],
                "status": area_status(area),
                "supported": sum(
                    member["weight"] for member in area["members"]
                    if member["status"] == "supported"
                ),
                "total": sum(member["weight"] for member in area["members"]),
                "evidence": sorted({
                    member["evidence"] for member in area["members"] if member.get("evidence")
                }),
            }
            for area in data["areas"]
        ],
    }


def print_table(result):
    print(f"Java mod compatibility: {result['percentage']:.1f}% "
          f"({result['supported_weight']}/{result['total_weight']} weighted members)")
    print(f"Minecraft {result['minecraft']} / {result['loader']}")
    print()
    print("area\tphase\tstatus\tweight\tevidence")
    for area in result["areas"]:
        evidence = ",".join(area["evidence"]) or "-"
        print(f"{area['id']}\t{area['phase']}\t{area['status']}\t"
              f"{area['supported']}/{area['total']}\t{evidence}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ledger", type=Path, default=DEFAULT_LEDGER)
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--check", action="store_true", help="validate without printing a table")
    args = parser.parse_args(argv)
    try:
        result = summary(load(args.ledger))
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"compat_ledger: {error}", file=sys.stderr)
        return 1
    if args.check:
        print(f"compat ledger valid: {result['supported_weight']}/{result['total_weight']}")
    elif args.json:
        print(json.dumps(result, indent=2))
    else:
        print_table(result)
    return 0


if __name__ == "__main__":
    sys.exit(main())
