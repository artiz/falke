use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use teloxide::prelude::*;
use tracing::{info, warn};
use uuid::Uuid;

use crate::polymarket::clob_api::ClobClient;
use crate::strategy::signals::SignalSource;
use crate::trading::engine::SharedSessions;
use crate::trading::portfolio::Position;

/// Reconcile locally-stored open positions against actual CLOB positions.
///
/// For each locally open position:
///   - If the CLOB reports size == 0 or the position is absent → it was resolved
///     or closed while the bot was offline.  We close it locally at the current
///     CLOB price (or 0 if unavailable) with reason "reconciled-on-restart".
///   - If the CLOB reports a different (non-zero) size → update local quantity
///     and cost_basis proportionally so we don't track phantom tokens.
///
/// CLOB positions that are NOT in the local portfolio are imported so they
/// appear in the trades view after a restart or --reset.
pub async fn reconcile_live_positions(clob: &ClobClient, sessions: &SharedSessions, bot: &Bot) {
    info!("Reconciling live positions with Polymarket CLOB...");

    let clob_positions = match clob.get_positions().await {
        Ok(p) => p,
        Err(e) => {
            warn!("Could not fetch CLOB positions for reconciliation: {e}");
            return;
        }
    };

    info!("CLOB reports {} open position(s)", clob_positions.len());

    // Build a lookup: token_id -> full ClobPosition
    let clob_map: HashMap<String, _> = clob_positions
        .into_iter()
        .map(|p| (p.asset_id.clone(), p))
        .collect();

    let mut closed_count: usize = 0;
    let mut updated_count: usize = 0;
    let mut imported_count: usize = 0;
    let mut imported_names: Vec<String> = Vec::new();

    let user_ids: Vec<i64>;
    {
        let mut sessions_lock = sessions.write().await;
        user_ids = sessions_lock.keys().cloned().collect();

        for portfolio in sessions_lock.values_mut() {
            // Token_ids currently open
            let local_token_ids: std::collections::HashSet<String> = portfolio
                .open_positions
                .values()
                .map(|p| p.token_id.clone())
                .collect();

            // Token_ids already closed in this session — don't re-import them.
            // This prevents duplicates when a sell order didn't fill on the CLOB but
            // the position was already closed locally (TP/SL fired in a previous run).
            let closed_token_ids: std::collections::HashSet<String> = portfolio
                .trade_history
                .iter()
                .map(|t| t.token_id.clone())
                .collect();

            // --- 1. Update / close existing local positions ---
            let position_ids: Vec<String> = portfolio.open_positions.keys().cloned().collect();

            for pos_id in position_ids {
                let pos = match portfolio.open_positions.get(&pos_id) {
                    Some(p) => p.clone(),
                    None => continue,
                };

                let clob_entry = clob_map.get(&pos.token_id);
                match clob_entry {
                    None => {
                        // Absent from CLOB — resolved or closed while offline.
                        // Use last known price: if it was near 1 the market resolved YES (win),
                        // if near 0 it resolved NO (loss). Avoids recording wins as $0 losses.
                        let exit_price = if pos.current_price >= dec!(0.95) {
                            Decimal::ONE
                        } else if pos.current_price <= dec!(0.05) {
                            Decimal::ZERO
                        } else {
                            pos.current_price
                        };
                        let reason = if exit_price >= Decimal::ONE {
                            "resolved-win"
                        } else if exit_price <= Decimal::ZERO {
                            "resolved-loss"
                        } else {
                            "reconciled-on-restart"
                        };
                        match portfolio.close_position(&pos_id, exit_price, reason) {
                            Ok(trade) => {
                                info!(
                                    "Reconciled: closed {} ({}) — absent from CLOB, exit={:.4} (pnl=${:.2})",
                                    pos.outcome_name, pos.token_id, exit_price, trade.realized_pnl
                                );
                                closed_count += 1;
                            }
                            Err(e) => warn!("Reconcile: failed to close position {pos_id}: {e}"),
                        }
                    }
                    Some(cp) if cp.size_f64() < 0.001 => {
                        // Size effectively zero
                        let exit_price = cp.cur_price;
                        match portfolio.close_position(&pos_id, exit_price, "reconciled-on-restart")
                        {
                            Ok(trade) => {
                                info!(
                                    "Reconciled: closed {} at ${:.4} — size=0 on CLOB (pnl=${:.2})",
                                    pos.outcome_name, exit_price, trade.realized_pnl
                                );
                                closed_count += 1;
                            }
                            Err(e) => warn!("Reconcile: failed to close position {pos_id}: {e}"),
                        }
                    }
                    Some(cp) => {
                        let clob_size = cp.size_f64();
                        let local_size: f64 = pos.quantity.to_string().parse().unwrap_or(0.0);
                        let diff = (clob_size - local_size).abs();

                        if diff > 0.01 {
                            if let Some(p) = portfolio.open_positions.get_mut(&pos_id) {
                                let ratio = clob_size / local_size.max(f64::EPSILON);
                                p.quantity = Decimal::try_from(clob_size).unwrap_or(p.quantity);
                                p.cost_basis =
                                    p.cost_basis * Decimal::try_from(ratio).unwrap_or(Decimal::ONE);
                                p.current_price = cp.cur_price;
                                info!(
                                    "Reconciled: updated {} qty {:.2} → {:.2}",
                                    pos.outcome_name, local_size, clob_size
                                );
                                updated_count += 1;
                            }
                        }
                    }
                }
            }

            // --- 2. Import CLOB positions not tracked locally ---
            for (token_id, cp) in &clob_map {
                if local_token_ids.contains(token_id) {
                    continue; // already handled above
                }
                if closed_token_ids.contains(token_id) {
                    continue; // already closed locally — sell order may still be resting on CLOB
                }
                if cp.size_f64() < 0.001 {
                    continue; // skip dust
                }

                let size = cp.size;
                let avg_price = cp.avg_price;
                let cost_basis = avg_price * size;

                // Shorten condition_id for display (first 8 chars)
                let short_id = if cp.condition_id.len() > 10 {
                    format!("{}…", &cp.condition_id[..8])
                } else {
                    cp.condition_id.clone()
                };
                let outcome_name = format!("Imported {short_id}");

                let pos = Position {
                    id: Uuid::new_v4().to_string(),
                    condition_id: cp.condition_id.clone(),
                    token_id: token_id.clone(),
                    question: format!("CLOB position (imported) — {short_id}"),
                    outcome_name: outcome_name.clone(),
                    side: "LONG".to_string(),
                    entry_price: avg_price,
                    quantity: size,
                    cost_basis,
                    current_price: cp.cur_price,
                    source: SignalSource::TailRisk,
                    opened_at: Utc::now(),
                    use_take_profit: true,
                    imported: true,
                    market_url: None,
                };

                // Import without charging balance (original cost is unknown).
                // initial_balance will be reset to total_value below so P&L starts clean.
                portfolio.open_positions.insert(pos.id.clone(), pos);

                info!(
                    "Reconciled: imported CLOB position {} size={:.2} avg_price={:.4}",
                    outcome_name, size, avg_price
                );
                imported_count += 1;
                imported_names.push(format!(
                    "{} (size={:.2}, entry={:.4})",
                    outcome_name, size, avg_price
                ));
            }

            // Deduplicate trade_history: remove entries with identical (token_id, entry, exit, qty).
            // These accumulate when positions are re-imported and re-closed across restarts.
            // Use prices/quantity (not realized_pnl) as key — Decimal scale can differ after arithmetic.
            {
                let mut seen = std::collections::HashSet::new();
                portfolio.trade_history.retain(|t| {
                    let key = format!(
                        "{};{:.4};{:.4};{:.4}",
                        t.token_id, t.entry_price, t.exit_price, t.quantity
                    );
                    seen.insert(key)
                });
            }

            // Reset P&L baseline to current total value so Session P&L starts at zero.
            // This avoids fake gains/losses from positions imported without known cost.
            if imported_count > 0 || closed_count > 0 || updated_count > 0 {
                portfolio.initial_balance = portfolio.total_value();
            }
        }
    }

    // --- 3. Send Telegram summary to all registered users ---
    let summary = build_summary(
        clob_map.len(),
        closed_count,
        updated_count,
        imported_count,
        &imported_names,
    );
    info!("{summary}");

    for user_id in user_ids {
        if let Err(e) = bot.send_message(ChatId(user_id), &summary).await {
            warn!("Reconcile: failed to notify user {user_id}: {e}");
        }
    }
}

fn build_summary(
    clob_total: usize,
    closed: usize,
    updated: usize,
    imported: usize,
    imported_names: &[String],
) -> String {
    let mut lines = vec![
        "Reconciliation complete".to_string(),
        format!("CLOB positions: {clob_total}"),
        format!("Closed (resolved offline): {closed}"),
        format!("Updated (size mismatch): {updated}"),
        format!("Imported (new): {imported}"),
    ];

    if !imported_names.is_empty() {
        lines.push(String::new());
        lines.push("Imported positions:".to_string());
        for name in imported_names {
            lines.push(format!("  • {name}"));
        }
    }

    if clob_total == 0 && closed == 0 && updated == 0 && imported == 0 {
        lines.push(String::new());
        lines.push("No open positions on CLOB.".to_string());
    }

    lines.join("\n")
}
