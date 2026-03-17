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
    let mut rows = vec![
        vec![
            InlineKeyboardButton::callback("Portfolio", "cmd:status"),
            InlineKeyboardButton::callback("Markets", "cmd:markets"),
        ],
        vec![InlineKeyboardButton::callback("Trades", "cmd:trades")],
    ];
    if testing_mode {
        rows.push(vec![InlineKeyboardButton::callback(
            "Test Results",
            "cmd:test",
        )]);
    }
    if is_live {
        rows.push(vec![
            InlineKeyboardButton::callback("Sell Trade", "ask:sell_trade"),
            InlineKeyboardButton::callback("Withdraw All", "ask:withdraw"),
        ]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "⚙️ Settings",
        "cmd:settings",
    )]);
    // Reset Session only in paper mode
    if !is_live {
        rows.push(vec![
            InlineKeyboardButton::callback("Reset Session", "ask:reset"),
            InlineKeyboardButton::callback(stop_label, stop_cmd),
        ]);
    } else {
        rows.push(vec![InlineKeyboardButton::callback(stop_label, stop_cmd)]);
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

/// Settings display text
pub fn settings_text(
    tp_pct: Decimal,
    bet_usd: Decimal,
    max_price: Decimal,
    window_hours: u32,
    paused: bool,
) -> String {
    let mode = if paused { "PAUSED" } else { "ACTIVE" };
    format!(
        "⚙️ Settings\nTP: {}% | Bet: ${} | Max Price: {:.1}c | Window: {}h | Mode: {}",
        tp_pct,
        bet_usd,
        max_price * dec!(100),
        window_hours,
        mode,
    )
}

/// Settings edit keyboard — inline buttons to tweak each parameter
pub fn settings_keyboard(paused: bool) -> InlineKeyboardMarkup {
    let (pause_label, pause_cb) = if paused {
        ("▶ Resume", "confirm:resume")
    } else {
        ("⏸ Pause", "confirm:stop")
    };
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("TP +5%", "settings:tp_up"),
            InlineKeyboardButton::callback("TP -5%", "settings:tp_down"),
            InlineKeyboardButton::callback("Bet +$1", "settings:bet_up"),
            InlineKeyboardButton::callback("Bet -$1", "settings:bet_down"),
        ],
        vec![
            InlineKeyboardButton::callback("Price +0.5c", "settings:price_up"),
            InlineKeyboardButton::callback("Price -0.5c", "settings:price_down"),
            InlineKeyboardButton::callback("Win +1h", "settings:window_up"),
            InlineKeyboardButton::callback("Win -1h", "settings:window_down"),
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
