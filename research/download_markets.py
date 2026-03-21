"""
Download expired Polymarket markets for a given date range.

Usage:
  python download_markets.py                         # today's markets
  python download_markets.py --date 2026-03-19       # specific day
  python download_markets.py --start 2026-03-01 --end 2026-03-20  # range

Output: data/<start>-<end>-markets.json

Each saved market has a normalized `tokens` list:
  [{"token_id": "...", "outcome": "Yes"}, ...]
"""

import argparse
import json
import time
from datetime import date
from pathlib import Path

import requests

GAMMA_API = "https://gamma-api.polymarket.com"
DATA_DIR = Path(__file__).parent / "data"
DATA_DIR.mkdir(exist_ok=True)


def parse_json_field(value) -> list:
    """Parse a field that may be a JSON-encoded string or already a list."""
    if isinstance(value, list):
        return value
    if isinstance(value, str):
        try:
            parsed = json.loads(value)
            return parsed if isinstance(parsed, list) else []
        except (json.JSONDecodeError, ValueError):
            return []
    return []


def normalize_market(m: dict) -> dict:
    """Add a normalized `tokens` list and `topic` field to each market."""
    token_ids = parse_json_field(m.get("clobTokenIds", []))
    outcomes = parse_json_field(m.get("outcomes", []))

    tokens = []
    for i, tid in enumerate(token_ids):
        outcome = outcomes[i] if i < len(outcomes) else f"Outcome {i}"
        tokens.append({"token_id": tid, "outcome": outcome})

    m["tokens"] = tokens
    m["topic"] = m.get("category") or m.get("groupItemTitle") or "unknown"
    return m


def safe_json_dump(data, fp, **kwargs):
    """Serialize data, normalizing any non-standard types (works around Python 3.14 json.dump changes)."""
    normalized = json.loads(json.dumps(data, default=str))
    fp.write(json.dumps(normalized, **kwargs))


def fetch_markets(end_date_min: str, end_date_max: str) -> list[dict]:
    """Fetch all closed markets that resolved within the given date range."""
    markets = []
    offset = 0
    limit = 100

    print(f"Fetching markets resolved between {end_date_min} and {end_date_max}...")

    while True:
        params = {
            "closed": "true",
            "end_date_min": end_date_min,
            "end_date_max": end_date_max,
            "limit": limit,
            "offset": offset,
        }
        resp = requests.get(f"{GAMMA_API}/markets", params=params, timeout=30)
        resp.raise_for_status()
        batch = resp.json()

        if not batch:
            break

        for m in batch:
            markets.append(normalize_market(m))

        print(f"  fetched {len(markets)} markets so far (offset={offset})")

        if len(batch) < limit:
            break

        offset += limit
        time.sleep(0.2)

    return markets


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--date", help="Single day (YYYY-MM-DD), defaults to today")
    parser.add_argument("--start", help="Start date (YYYY-MM-DD)")
    parser.add_argument("--end", help="End date (YYYY-MM-DD)")
    args = parser.parse_args()

    if args.start and args.end:
        start_str = args.start
        end_str = args.end
    elif args.date:
        start_str = args.date
        end_str = args.date
    else:
        today = date.today().isoformat()
        start_str = today
        end_str = today

    markets = fetch_markets(start_str, end_str)
    print(f"\nTotal markets: {len(markets)}")

    binary = [m for m in markets if len(m["tokens"]) == 2]
    multi = [m for m in markets if len(m["tokens"]) > 2]
    no_tokens = [m for m in markets if len(m["tokens"]) == 0]
    print(f"  Binary: {len(binary)}, Multi-outcome: {len(multi)}, No tokens: {len(no_tokens)}")

    token_ids = [t["token_id"] for m in markets for t in m["tokens"] if t["token_id"]]
    print(f"  Total tokens: {len(token_ids)}")

    out_path = DATA_DIR / f"{start_str}-{end_str}-markets.json"

    if out_path.exists():
        print(f"\n{out_path} already exists — skipping download.")
        print("Delete the file to re-download.")
        return

    with open(out_path, "w") as f:
        safe_json_dump(markets, f, indent=2)

    print(f"\nSaved to {out_path}")


if __name__ == "__main__":
    main()
