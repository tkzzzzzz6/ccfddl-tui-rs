"""Generate golden data from the original Python implementation.

Run from the Python repo root so `ccfddl_tui` is importable:

    cd d:\\develop\\tui
    .venv\\Scripts\\python.exe d:\\develop\\ccfddl-tui-rs\\tests\\fixtures\\gen_golden.py

Writes golden.json next to this script. `assets/allconf.yml` must be the same
snapshot the Rust tests embed.
"""

import json
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

import yaml

from ccfddl_tui.data.data_service import DataService
from ccfddl_tui.models import Conference
from ccfddl_tui.utils.timezone import format_duration

HERE = Path(__file__).resolve().parent
SNAPSHOT = HERE.parent.parent / "assets" / "allconf.yml"
NOW = datetime(2026, 10, 22, 11, 34, 38, tzinfo=timezone.utc)
QUERIES = ["cvpr", "Neur", "sigmod", "ICDE", "vision", "zzzzqq", "ai", "acm", "neurpis"]
DURATIONS = [
    1, 59, 3600, 86399, 86400, 90061, 29 * 86400 + 5, 30 * 86400, 31 * 86400,
    59 * 86400, 60 * 86400, 65 * 86400 + 7200, 400 * 86400,
]


def row_to_json(row):
    return {
        "title": row.title,
        "year": row.year,
        "sub": row.sub,
        "rank": row.rank,
        "core_rank": row.core_rank,
        "thcpl_rank": row.thcpl_rank,
        "deadline": row.deadline.astimezone(timezone.utc).isoformat() if row.deadline else None,
        "countdown": row.countdown,
        "place": row.place,
        "date": row.date,
        "link": row.link,
        "is_running": row.is_running,
        "is_tbd": row.is_tbd,
        "is_expired": row.is_expired,
    }


def main() -> int:
    data = yaml.safe_load(SNAPSHOT.read_text(encoding="utf-8"))
    service = DataService()
    service._favorites = set()
    service._conferences = [Conference.from_dict(c) for c in data]

    rows = service.process_rows(NOW)
    ordered = service.sort_rows(rows)

    searches = {
        query: [f"{r.title}_{r.year}" for r in service.fuzzy_search(rows, query)]
        for query in QUERIES
    }
    durations = [
        [seconds, format_duration(NOW + timedelta(seconds=seconds), NOW)]
        for seconds in DURATIONS
    ]

    out = {
        "now": NOW.isoformat(),
        "rows_unsorted": [row_to_json(r) for r in rows],
        "rows_sorted_keys": [f"{r.title}_{r.year}" for r in ordered],
        "searches": searches,
        "durations": durations,
    }
    (HERE / "golden.json").write_text(
        json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8"
    )
    print(f"wrote golden.json: {len(rows)} rows")
    return 0


if __name__ == "__main__":
    sys.exit(main())
