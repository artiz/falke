# Falke — Polymarket Arbitrage + Momentum Trading Bot

Rust-based automated trading bot for Polymarket prediction markets. Combines 50% arbitrage (cross-outcome mispricing) and 50% momentum/derivative-based scalping. Controlled via Telegram bot with phone-number-gated access. Paper trading mode for strategy validation.

## Prerequisites

- Rust 1.91+ (`rustup update stable`)
- AWS account (for production deployment)
- Telegram account
- Polymarket account with Polygon wallet (for live trading)

## TODO:

* Change market_expiry_window_hours type to decimal to play with 0.1/5 MR strategy in 30 min
window
* 

## Getting Telegram Bot Token

1. Open Telegram and search for **@BotFather**
2. Start a chat and send `/newbot`
3. Choose a display name (e.g., "Falke Trading Bot")
4. Choose a username — must end in `bot` (e.g., `falke_trading_bot`)
5. BotFather replies with your token:
   ```
   Use this token to access the HTTP API:
   7123456789:AAH1234567890abcdefghijklmnopqrstuv
   ```
6. Copy this token — this is your `TELEGRAM_BOT_TOKEN`
7. Optionally, send `/setcommands` to BotFather, select your bot, then paste:
   ```
   start - Register and start using Falke
   status - View portfolio and P&L
   strategy - Configure strategy parameters
   markets - Browse monitored markets
   trades - View trade history
   mode - Switch between paper/live trading
   stop - Pause all trading
   ```

## Getting Your Phone Number for Access Control

The bot restricts registration to pre-approved phone numbers. Set `ALLOWED_PHONES` in `.env` to your phone number with country code:

```
ALLOWED_PHONES=+43123456789
```

Multiple numbers can be comma-separated: `+43123456789,+49987654321`

## Polymarket API Access

- **Gamma API** (market data): Public, no auth needed. Base URL: `https://gamma-api.polymarket.com`
- **CLOB API** (order placement): Requires Ethereum wallet signature
  1. Your Polymarket account is tied to a Polygon wallet
  2. If using MetaMask: Settings → Security & Privacy → Export Private Key
  3. The bot derives CLOB API credentials automatically via EIP-712 signing
  4. Set `WALLET_PRIVATE_KEY` in `.env` (only needed for live trading)

## Quick Start (Local Development)

```bash
# 1. Clone and enter project
cd falke

# 2. Copy env template and fill in values
cp .env.example .env
# Edit .env: set TELEGRAM_BOT_TOKEN and ALLOWED_PHONES at minimum

# 3. Run in paper trading mode (default)
cargo run
```

The bot starts with:
- Paper trading mode, $1000 virtual balance
- 50/50 arbitrage/momentum strategy split
- Monitoring markets expiring in 1-3 days
- 10-second price polling interval

## Configuration

All config is via environment variables (or `.env` file). See `.env.example` for all options.

| Variable | Default | Description |
|----------|---------|-------------|
| `TRADING_MODE` | `paper` | `paper` or `live` |
| `PAPER_BALANCE` | `1000.0` | Initial paper trading balance (USD) |
| `TELEGRAM_BOT_TOKEN` | required | From BotFather |
| `ALLOWED_PHONES` | required | Comma-separated phone numbers |
| `ARB_THRESHOLD` | `0.97` | Sum of outcome prices below this = arb signal |
| `ARB_BUDGET_PCT` | `0.50` | Fraction of balance for arbitrage |
| `MOMENTUM_DERIVATIVE_THRESHOLD` | `0.30` | 30% price change in 5 min = momentum signal |
| `MOMENTUM_WINDOW_SEC` | `300` | Derivative calculation window (seconds) |
| `MOMENTUM_BUDGET_PCT` | `0.50` | Fraction of balance for momentum |
| `MOMENTUM_POLL_INTERVAL_SEC` | `10` | Price polling frequency |
| `MARKET_EXPIRY_WINDOW_DAYS` | `3` | Only monitor markets expiring within N days |
| `MIN_LIQUIDITY_USD` | `1000.0` | Skip markets below this liquidity |
| `MAX_BET_USD` | `50.0` | Max single bet size |
| `MAX_OPEN_POSITIONS` | `20` | Max concurrent positions |
| `COOLDOWN_SEC` | `600` | Cooldown per market after a trade |
| `WALLET_PRIVATE_KEY` | empty | Polygon wallet key (live trading only) |
| `AWS_REGION` | `eu-west-2` | AWS region for DynamoDB |
| `DYNAMO_TABLE_PREFIX` | `falke` | DynamoDB table name prefix |

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                    Falke Bot Process                     │
│                                                          │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────┐   │
│  │  Telegram   │  │  Market Data │  │ Trading Engine │   │
│  │  Bot        │  │  Collector   │  │                │   │
│  │  (teloxide) │  │  (polling)   │  │  Arb Scanner   │   │
│  │             │  │              │  │  Mom Scanner   │   │
│  │  /start     │  │  Gamma API ──┤  │  Risk Manager  │   │
│  │  /status    │  │  Price Store │  │  Paper Engine  │   │
│  │  /markets   │  │  (ring buf)  │  │  Live Executor │   │
│  │  /trades    │  │              │  │                │   │
│  └──────┬──────┘  └──────┬───────┘  └───────┬────────┘   │
│         │                │                  │            │
│         └────────┬───────┴──────────────────┘            │
│                  │                                       │
│         ┌────────▼────────┐                              │
│         │  Shared State   │                              │
│         │  (Arc<RwLock>)  │                              │
│         │                 │                              │
│         │  - MarketData   │                              │
│         │  - Sessions     │                              │
│         │  - Portfolios   │                              │
│         └─────────────────┘                              │
└──────────────────────────────────────────────────────────┘

External:
  Polymarket Gamma API ← market data (public)
  Polymarket CLOB API  ← order placement (authenticated)
  Telegram Bot API     ← user interaction
  AWS DynamoDB         ← persistence (production)
```

## Project Structure

```
src/
├── main.rs                  # Entrypoint — starts all services concurrently
├── config.rs                # ENV-based config with defaults
├── error.rs                 # Unified error types (FalkeError)
├── db/
│   ├── models.rs            # User, TradeRecord structs
│   └── dynamo.rs            # DynamoDB client wrapper
├── telegram/
│   ├── bot.rs               # Bot startup, long-polling message loop
│   ├── handlers.rs          # Command handlers (/start, /status, etc.)
│   ├── keyboards.rs         # Inline keyboard builders (menus, buttons)
│   └── auth.rs              # Phone number allowlist verification
├── polymarket/
│   ├── gamma_api.rs         # Market listing, price fetching (public API)
│   ├── clob_api.rs          # Order book reading, order placement
│   ├── types.rs             # API response types, internal market types
│   └── auth.rs              # EIP-712 wallet signing for CLOB credentials
├── strategy/
│   ├── arbitrage.rs         # Cross-outcome arb: sum of prices < threshold
│   ├── momentum.rs          # 5-min derivative: |pct_change| > 30% threshold
│   ├── signals.rs           # Signal types (Arb/Momentum), metadata
│   └── risk.rs              # Position sizing, exposure limits, cooldowns
├── trading/
│   ├── engine.rs            # Main loop: scan signals → risk check → execute
│   ├── paper.rs             # Paper trading: simulated fills + slippage
│   ├── executor.rs          # Live trading: real CLOB orders
│   └── portfolio.rs         # Positions, P&L tracking, balance management
├── market_data/
│   ├── collector.rs         # Polls Gamma API, updates tracked markets
│   ├── price_store.rs       # In-memory ring buffer, linear regression derivative
│   └── websocket.rs         # Optional WebSocket price stream
└── utils/
    └── crypto.rs            # Wallet parsing utilities
```

## Key Design Decisions

### Three Concurrent Tasks

`main.rs` spawns three tokio tasks:
1. **Market Data Collector** — polls Gamma API every 10s, updates shared price store
2. **Trading Engine** — scans for arb/momentum signals, executes trades per user session
3. **Telegram Bot** — handles user commands via long-polling

All share state through `Arc<RwLock<T>>`.

### Paper vs Live Trading

- **Paper mode** (default): Orders fill instantly at current price + 1% simulated slippage. No real money involved. Full P&L tracking.
- **Live mode**: Orders go through Polymarket CLOB API as Fill-or-Kill. Requires wallet private key.

### Arbitrage Strategy

For each tracked market, sum all outcome prices. If `sum < 0.97` (configurable), buy all outcomes to lock in the spread. Edge = `(1 - sum) / sum`.

### Momentum Strategy

For each outcome token, compute a linear regression slope over the last 5 minutes of price data. If the absolute percentage change exceeds 30% (configurable):
- Price rising fast → buy YES
- Price falling fast → buy NO

### Risk Management

- Max single bet: $50 (configurable)
- Max open positions: 20
- Per-market cooldown: 10 minutes after a trade
- Budget split enforced: arb trades draw from arb budget, momentum from momentum budget

### Multi-User Support

Each Telegram user gets an isolated portfolio with own balance, positions, and trade history. Market data collection is shared across all users.

### Phone Number Gating

On `/start`, the bot requests the user's phone via Telegram's contact-sharing button. Only numbers in the `ALLOWED_PHONES` allowlist can register.

## AWS Deployment

### Infrastructure (Terraform)

```bash
cd terraform
cp terraform.tfvars.example terraform.tfvars
# Edit terraform.tfvars with your values

terraform init
terraform plan
terraform apply
```

Creates:
- **VPC** with public/private subnets and NAT gateway
- **ECS Fargate** cluster + service (single task, 0.25 vCPU / 512 MB)
- **DynamoDB** tables: `falke-dev-users`, `falke-dev-trades` (on-demand billing)
- **Secrets Manager** for bot token, phone numbers, wallet key
- **ECR** repository for Docker image
- **CloudWatch** logs, dashboard, and task-count alarm

### Deploy a New Version

```bash
# Build and push Docker image
aws ecr get-login-password --region eu-west-2 | docker login --username AWS --password-stdin <ECR_URL>
docker build -t falke .
docker tag falke:latest <ECR_URL>:latest
docker push <ECR_URL>:latest

# Force new deployment
aws ecs update-service --cluster falke-dev --service falke-dev --force-new-deployment --region eu-west-2

# View logs
aws logs tail /ecs/falke-dev --follow --region eu-west-2
```

## Telegram Bot Commands

| Command | Description |
|---------|-------------|
| `/start` | Register (phone verification required) |
| `/status` | Portfolio summary: balance, P&L, open positions |
| `/markets` | List tracked markets with prices |
| `/trades` | Recent open and closed trades |
| `/strategy` | Change arb/momentum allocation |
| `/mode` | Switch between paper and live trading |
| `/stop` | Pause all trading |

## Development

```bash
# Check compilation
cargo check

# Run tests
cargo test

# Run with verbose logging
RUST_LOG=falke=debug cargo run

# Run with custom config
TRADING_MODE=paper PAPER_BALANCE=500 MOMENTUM_DERIVATIVE_THRESHOLD=0.20 cargo run
```

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `teloxide` | Telegram bot framework |
| `reqwest` | HTTP client for Polymarket APIs |
| `tokio` | Async runtime |
| `ethers` | Ethereum wallet signing (CLOB auth) |
| `aws-sdk-dynamodb` | DynamoDB persistence |
| `rust_decimal` | Precise decimal arithmetic for prices |
| `tokio-tungstenite` | WebSocket client |
| `tracing` | Structured logging |

## Status

### Implemented (Phase 1)
- Full project scaffold with all modules
- Telegram bot with phone-gated registration and 7 commands
- Gamma API client for market data fetching
- CLOB API client for order placement
- In-memory price store with ring buffer and linear regression derivative
- Arbitrage signal detection (cross-outcome price sum)
- Momentum signal detection (5-min derivative threshold)
- Risk manager (position sizing, limits, cooldowns)
- Paper trading engine with simulated slippage
- Portfolio tracking with per-user P&L
- DynamoDB persistence layer
- Terraform infrastructure (VPC, ECS, DynamoDB, Secrets Manager, CloudWatch)
- Dockerfile for containerized deployment

### TODO (Phase 2)
- Wire DynamoDB persistence into the trading engine (currently in-memory only)
- Implement callback query handling for inline keyboard buttons
- Add WebSocket price streaming alongside polling
- Cross-platform arbitrage (Polymarket vs Kalshi)
- Position auto-close on market resolution
- Trailing stop-loss for momentum positions
- Daily/weekly P&L report via Telegram
- Backtesting mode with historical data
- Rate limiting and circuit breaker for API calls
