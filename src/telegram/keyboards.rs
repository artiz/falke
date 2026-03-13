use teloxide::types::{
    InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup, KeyboardRemove,
    ReplyMarkup,
};

/// Request phone number sharing (for registration)
pub fn phone_request_keyboard() -> KeyboardMarkup {
    KeyboardMarkup::new(vec![vec![KeyboardButton::new("Share Phone Number")
        .request(teloxide::types::ButtonRequest::Contact)]])
    .resize_keyboard()
    .one_time_keyboard()
}

/// Remove custom keyboard
pub fn remove_keyboard() -> ReplyMarkup {
    ReplyMarkup::KeyboardRemove(KeyboardRemove::new())
}

/// Main menu after registration
pub fn main_menu() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("Portfolio", "cmd:status"),
            InlineKeyboardButton::callback("Markets", "cmd:markets"),
        ],
        vec![
            InlineKeyboardButton::callback("Trades", "cmd:trades"),
            InlineKeyboardButton::callback("Strategy", "cmd:strategy"),
        ],
        vec![
            InlineKeyboardButton::callback("Mode", "cmd:mode"),
            InlineKeyboardButton::callback("Stop", "cmd:stop"),
        ],
    ])
}

/// Strategy configuration
/// Labels: Arb/Mom/MR/Tail
pub fn strategy_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("10/25/25/20", "strategy:balanced"),
            InlineKeyboardButton::callback("10/35/25/10", "strategy:mom_heavy"),
        ],
        vec![
            InlineKeyboardButton::callback("10/15/35/20", "strategy:mr_heavy"),
            InlineKeyboardButton::callback("10/15/15/40", "strategy:tail_heavy"),
        ],
        vec![InlineKeyboardButton::callback(
            "Back to Menu",
            "cmd:menu",
        )],
    ])
}

/// Trading mode switch
pub fn mode_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("Paper Trading", "mode:paper"),
            InlineKeyboardButton::callback("Live Trading", "mode:live"),
        ],
        vec![InlineKeyboardButton::callback(
            "Back to Menu",
            "cmd:menu",
        )],
    ])
}

/// Stop menu with stop + reset options
pub fn stop_menu() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("Pause Trading", "confirm:stop"),
        ],
        vec![
            InlineKeyboardButton::callback("Reset Paper Session", "ask:reset"),
        ],
        vec![
            InlineKeyboardButton::callback("Back to Menu", "cmd:menu"),
        ],
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
