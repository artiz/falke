"""
Download 72h price history for all tokens in a markets file.

Usage:
  python download_prices.py                                       # today's markets file
  python download_prices.py --markets data/2026-03-19-2026-03-19-markets.json
  python download_prices.py --markets data/2026-03-01-2026-03-20-markets.json --window 72

Output: data/<prefix>-prices.json  (same prefix as markets file)

Resumable: already-downloaded token IDs are skipped.
"""

import argparse
import json
import time
from datetime import date
from pathlib import Path

import requests

CLOB_API = "https://clob.polymarket.com"
DATA_DIR = Path(__file__).parent / "data"


def load_markets(markets_path: Path) -> list[dict]:
    with open(markets_path) as f:
        return json.load(f)


def load_existing_prices(prices_path: Path) -> dict[str, list]:
    if prices_path.exists():
        try:
            with open(prices_path) as f:
                return json.load(f)
        except json.JSONDecodeError as e:
            print(f"Warning: checkpoint file corrupted ({e}), starting fresh.")
            prices_path.rename(prices_path.with_suffix(".corrupted.json"))
    return {}


def save_prices(prices: dict, path: Path):
    # Atomic write: write to temp file then rename to avoid corruption on crash
    tmp = path.with_suffix(".tmp.json")
    with open(tmp, "w") as f:
        f.write(json.dumps(prices))
    tmp.replace(path)


def fetch_price_history(token_id: str, window_hours: int = 72) -> list[dict]:
    """
    Fetch price history for a token from CLOB API.
    Returns list of {t: timestamp_sec, p: price} dicts.

    Valid intervals: 1h, 6h, 1d, 1w, max  (72h not supported — use 1w).
    """
    # Map requested window to smallest valid interval that covers it
    if window_hours <= 1:
        interval = "1h"
    elif window_hours <= 6:
        interval = "6h"
    elif window_hours <= 24:
        interval = "1d"
    else:
        interval = "1w"  # covers up to 7 days

    params = {
        "market": token_id,
        "interval": interval,
        "fidelity": 60,  # 1-minute candles
    }
    resp = requests.get(
        f"{CLOB_API}/prices-history", params=params, timeout=30
    )
    resp.raise_for_status()
    data = resp.json()
    return data.get("history", [])


def collect_tokens(markets: list[dict], min_volume: float = 0.0) -> list[tuple[str, str, str]]:
    """Returns list of (condition_id, token_id, outcome) tuples, filtered by volume.

    Uses volumeNum as the activity filter (liquidityNum is always 0 for resolved markets).
    """
    tokens = []
    skipped = 0
    for m in markets:
        volume = float(m.get("volumeNum") or m.get("volume") or 0)
        if volume < min_volume:
            skipped += 1
            continue
        cid = m.get("conditionId", "")
        for t in m.get("tokens", []):
            token_id = t.get("token_id") or t.get("tokenId", "")
            outcome = t.get("outcome", "")
            if token_id:
                tokens.append((cid, token_id, outcome))
    if skipped:
        print(f"Skipped {skipped} markets below min volume ${min_volume:,.0f}")
    return tokens


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--markets",
        help="Path to markets JSON file (defaults to today's file)",
    )
    parser.add_argument(
        "--window", type=int, default=72, help="Price history window in hours"
    )
    parser.add_argument(
        "--min-volume", type=float, default=1.0,
        help="Skip markets with total volume below this USD value (default: 1, filters zero-volume markets)"
    )
    args = parser.parse_args()

    if args.markets:
        markets_path = Path(args.markets)
    else:
        # Auto-detect the latest markets file
        candidates = sorted(DATA_DIR.glob("*-markets.json"))
        if not candidates:
            print("No markets files found in data/. Run download_markets.py first.")
            return
        markets_path = candidates[-1]
        print(f"Auto-detected: {markets_path.name}")

    if not markets_path.exists():
        print(f"Markets file not found: {markets_path}")
        print("Run download_markets.py first.")
        return

    # Output file: same prefix as markets file, suffix -prices.json
    prefix = markets_path.stem.replace("-markets", "")
    prices_path = DATA_DIR / f"{prefix}-prices.json"

    markets = load_markets(markets_path)
    tokens = collect_tokens(markets, min_volume=args.min_volume)
    print(f"Found {len(tokens)} tokens across {len(markets)} markets")

    prices = load_existing_prices(prices_path)
    already_done = set(prices.keys())
    to_fetch = [(cid, tid, outcome) for cid, tid, outcome in tokens if tid not in already_done]
    print(f"Already downloaded: {len(already_done)}, remaining: {len(to_fetch)}")

    for i, (cid, token_id, outcome) in enumerate(to_fetch):
        try:
            history = fetch_price_history(token_id, args.window)
            prices[token_id] = {
                "condition_id": cid,
                "outcome": outcome,
                "history": history,
            }
            print(f"[{i+1}/{len(to_fetch)}] {token_id[:16]}... ({outcome}) — {len(history)} points")
        except Exception as e:
            print(f"[{i+1}/{len(to_fetch)}] ERROR {token_id[:16]}...: {e}")
            prices[token_id] = {
                "condition_id": cid,
                "outcome": outcome,
                "history": [],
                "error": str(e),
            }

        # Save incrementally every 20 tokens
        if (i + 1) % 20 == 0:
            save_prices(prices, prices_path)
            print(f"  → checkpoint saved ({len(prices)} tokens)")

        time.sleep(0.1)

    save_prices(prices, prices_path)
    print(f"\nDone. Saved {len(prices)} tokens to {prices_path}")

    # Summary
    with_data = sum(1 for v in prices.values() if v.get("history"))
    errors = sum(1 for v in prices.values() if v.get("error"))
    print(f"  With data: {with_data}, Errors: {errors}")


if __name__ == "__main__":
    main()
