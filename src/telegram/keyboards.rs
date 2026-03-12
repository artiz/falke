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
pub fn strategy_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("50/50 (Default)", "strategy:balanced"),
            InlineKeyboardButton::callback("70% Arb / 30% Mom", "strategy:arb_heavy"),
        ],
        vec![
            InlineKeyboardButton::callback("30% Arb / 70% Mom", "strategy:mom_heavy"),
            InlineKeyboardButton::callback("100% Arb Only", "strategy:arb_only"),
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

/// Confirmation keyboard
pub fn confirm_keyboard(action: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("Confirm", format!("confirm:{action}")),
        InlineKeyboardButton::callback("Cancel", "cmd:menu"),
    ]])
}
