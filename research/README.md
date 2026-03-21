# Falke MR Strategy Research

Scripts to backtest the Mean Reversion strategy on historical Polymarket data and train an ML signal filter.

## Setup

```bash
pip install -r requirements.txt
```

## Workflow

### 1. Download expired markets

```bash
# Today's markets (default)
python download_markets.py

# Specific day
python download_markets.py --date 2026-03-19

# Date range (recommended: 2+ weeks for ML training)
python download_markets.py --start 2026-03-01 --end 2026-03-20
```

Output: `data/<start>-<end>-markets.json`

### 2. Download price history

```bash
# Auto-detects the latest markets file
python download_prices.py

# Or specify explicitly
python download_prices.py --markets data/2026-03-01-2026-03-20-markets.json

# Custom window (default 72h)
python download_prices.py --window 48
```

Output: `data/<prefix>-prices.json`
Resumable — already-downloaded tokens are skipped on re-run.

### 3. Run analysis notebook

```bash
jupyter notebook analysis.ipynb
```

The notebook:
- Loads all available data files
- Simulates MR signals (replicating `src/strategy/mean_reversion.rs` exactly)
- Grid-searches `threshold × window_hours`
- Engineers features: pct_change, entry_price, liquidity, volatility, topic, market_type
- Trains XGBoost + Random Forest classifiers
- Compares ML-filtered vs naive MR PnL
- Exports best model to ONNX (`mr_classifier_*.onnx`)

## ONNX Integration in Rust

Add to `Cargo.toml`:
```toml
ort = { version = "2", features = ["load-dynamic"] }
```

Usage sketch:
```rust
let session = ort::Session::builder()?
    .with_model_from_file("mr_classifier.onnx")?;
let features = ndarray::array![[pct_change, entry_price, ...]];
let outputs = session.run(ort::inputs![features]?)?;
let win_prob: f32 = outputs[1].try_extract_tensor::<f32>()?[[0, 1]];
if win_prob > 0.60 { /* take the trade */ }
```

## Feature List (order matters for ONNX)

See `mr_classifier_meta.json` after running the notebook.
