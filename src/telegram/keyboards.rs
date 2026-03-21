use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use teloxide::types::{
    InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup, KeyboardRemove,
    ReplyMarkup,
};

/// Request phone number sharing (for registration)
pub fn phone_request_keyboard() -> KeyboardMarkup {
    KeyboardMarkup::new(vec![vec![
        KeyboardButton::new("Share Phone Number").request(teloxide::types::ButtonRequest::Contact)
    ]])
    .resize_keyboard()
    .one_time_keyboard()
}

/// Remove custom keyboard
pub fn remove_keyboard() -> ReplyMarkup {
    ReplyMarkup::KeyboardRemove(KeyboardRemove::new())
}

pub fn main_menu_with_state(
    paused: bool,
    testing_mode: bool,
    is_live: bool,
) -> InlineKeyboardMarkup {
    let (stop_label, stop_cmd) = if paused {
        ("▶ Resume", "confirm:resume")
    } else {
        ("⏸ Stop", "cmd:stop")
    };
    let mut rows = vec![vec![
        InlineKeyboardButton::callback("Portfolio", "cmd:status"),
        InlineKeyboardButton::callback("Markets", "cmd:markets"),
    ]];
    rows.push(vec![
        InlineKeyboardButton::callback("Trades", "cmd:trades"),
        InlineKeyboardButton::callback("⚙️ Settings", "cmd:settings"),
    ]);
    if is_live {
        rows.push(vec![
            InlineKeyboardButton::callback("Sell Trade", "ask:sell_trade"),
            InlineKeyboardButton::callback("Withdraw All", "ask:withdraw"),
        ]);
    }

    // Reset Session only in paper mode
    if !is_live {
        rows.push(vec![
            InlineKeyboardButton::callback("Reset Session", "ask:reset"),
            InlineKeyboardButton::callback(stop_label, stop_cmd),
        ]);
    } else {
        rows.push(vec![InlineKeyboardButton::callback(stop_label, stop_cmd)]);
    }

    if testing_mode {
        rows.push(vec![InlineKeyboardButton::callback(
            "Test Results",
            "cmd:test",
        )]);
    }

    InlineKeyboardMarkup::new(rows)
}

/// Trading mode switch
#[allow(dead_code)]
pub fn mode_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("Paper Trading", "mode:paper"),
            InlineKeyboardButton::callback("Live Trading", "mode:live"),
        ],
        vec![InlineKeyboardButton::callback("Back to Menu", "cmd:menu")],
    ])
}

/// Stop menu with stop + reset options
pub fn stop_menu(is_live: bool) -> InlineKeyboardMarkup {
    let mut rows = vec![vec![InlineKeyboardButton::callback(
        "Pause Trading",
        "confirm:stop",
    )]];
    if !is_live {
        rows.push(vec![InlineKeyboardButton::callback(
            "Reset Paper Session",
            "ask:reset",
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "Back to Menu",
        "cmd:menu",
    )]);
    InlineKeyboardMarkup::new(rows)
}

/// Confirmation keyboard for reset
pub fn confirm_reset_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("Yes, Reset Everything", "confirm:reset"),
        InlineKeyboardButton::callback("Cancel", "cmd:menu"),
    ]])
}

/// Settings display text — strategy-aware
pub fn settings_text(
    window_hours: u32,
    max_positions: usize,
    paused: bool,
    mr_budget_pct: Decimal,
    mr_threshold: Decimal,
    mr_bet_usd: Decimal,
) -> String {
    let mode = if paused { "PAUSED" } else { "ACTIVE" };
    let strategy_line = if mr_budget_pct >= dec!(1.0) {
        format!(
            "Strategy: MR (thr={:.0}% bet=${})",
            mr_threshold * dec!(100),
            mr_bet_usd,
        )
    } else if mr_budget_pct <= Decimal::ZERO {
        "Strategy: ML only".to_string()
    } else {
        format!(
            "Strategy: ML + MR {:.0}% (thr={:.0}% bet=${})",
            mr_budget_pct * dec!(100),
            mr_threshold * dec!(100),
            mr_bet_usd,
        )
    };
    format!(
        "⚙️ Settings\n{strategy_line}\nWindow: {window_hours}h | Positions: {max_positions} | Mode: {mode}",
    )
}

/// Settings edit keyboard — strategy-aware buttons
pub fn settings_keyboard(paused: bool, is_mr_mode: bool) -> InlineKeyboardMarkup {
    let (pause_label, pause_cb) = if paused {
        ("▶ Resume", "confirm:resume")
    } else {
        ("⏸ Pause", "confirm:stop")
    };
    // Row 1: bet and price/threshold controls (depend on active strategy)
    let row1 = if is_mr_mode {
        vec![
            InlineKeyboardButton::callback("MR Bet +$1", "settings:mr_bet_up"),
            InlineKeyboardButton::callback("MR Bet -$1", "settings:mr_bet_down"),
            InlineKeyboardButton::callback("Thr +5%", "settings:mr_thr_up"),
            InlineKeyboardButton::callback("Thr -5%", "settings:mr_thr_down"),
        ]
    } else {
        vec![
            InlineKeyboardButton::callback("TR Bet +$1", "settings:bet_up"),
            InlineKeyboardButton::callback("TR Bet -$1", "settings:bet_down"),
            InlineKeyboardButton::callback("Price +0.5c", "settings:price_up"),
            InlineKeyboardButton::callback("Price -0.5c", "settings:price_down"),
        ]
    };
    InlineKeyboardMarkup::new(vec![
        row1,
        vec![
            InlineKeyboardButton::callback("Win +1h", "settings:window_up"),
            InlineKeyboardButton::callback("Win -1h", "settings:window_down"),
            InlineKeyboardButton::callback("Pos +10", "settings:positions_up"),
            InlineKeyboardButton::callback("Pos -10", "settings:positions_down"),
        ],
        vec![
            InlineKeyboardButton::callback(pause_label, pause_cb),
            InlineKeyboardButton::callback("↩ Back", "cmd:menu"),
        ],
    ])
}

/// Confirmation keyboard
#[allow(dead_code)]
pub fn confirm_keyboard(action: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("Confirm", format!("confirm:{action}")),
        InlineKeyboardButton::callback("Cancel", "cmd:menu"),
    ]])
}
