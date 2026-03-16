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

pub fn main_menu_with_state(paused: bool, testing_mode: bool, is_live: bool) -> InlineKeyboardMarkup {
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
        vec![
            InlineKeyboardButton::callback("Trades", "cmd:trades"),
        ],
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
    rows.push(vec![
        InlineKeyboardButton::callback("Reset Session", "ask:reset"),
        InlineKeyboardButton::callback(stop_label, stop_cmd),
    ]);
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
pub fn stop_menu() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "Pause Trading",
            "confirm:stop",
        )],
        vec![InlineKeyboardButton::callback(
            "Reset Paper Session",
            "ask:reset",
        )],
        vec![InlineKeyboardButton::callback("Back to Menu", "cmd:menu")],
    ])
}

/// Confirmation keyboard for reset
pub fn confirm_reset_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("Yes, Reset Everything", "confirm:reset"),
        InlineKeyboardButton::callback("Cancel", "cmd:menu"),
    ]])
}

/// Confirmation keyboard
#[allow(dead_code)]
pub fn confirm_keyboard(action: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("Confirm", format!("confirm:{action}")),
        InlineKeyboardButton::callback("Cancel", "cmd:menu"),
    ]])
}
