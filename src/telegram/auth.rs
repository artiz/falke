use std::collections::HashSet;
use tracing::info;

/// Phone-number-based authorization.
/// Only phone numbers in the allowlist can register.
pub struct PhoneAuth {
    allowed_phones: HashSet<String>,
}

impl PhoneAuth {
    pub fn new(allowed_phones: HashSet<String>) -> Self {
        info!(
            "Phone auth initialized with {} allowed numbers",
            allowed_phones.len()
        );
        Self { allowed_phones }
    }

    /// Check if a phone number is authorized.
    /// Normalizes the number by stripping spaces and dashes.
    pub fn is_authorized(&self, phone: &str) -> bool {
        let normalized = normalize_phone(phone);
        self.allowed_phones
            .iter()
            .any(|allowed| normalize_phone(allowed) == normalized)
    }
}

/// Normalize a phone number by removing spaces, dashes, and parentheses
fn normalize_phone(phone: &str) -> String {
    phone
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phone_auth() {
        let mut allowed = HashSet::new();
        allowed.insert("+43 123 456 7890".to_string());

        let auth = PhoneAuth::new(allowed);

        assert!(auth.is_authorized("+431234567890"));
        assert!(auth.is_authorized("+43 123 456 7890"));
        assert!(auth.is_authorized("+43-123-456-7890"));
        assert!(!auth.is_authorized("+44999999999"));
    }
}
