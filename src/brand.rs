//! Product names. Each script gets its own name, and wherever there's room
//! all three are shown side by side.

pub const NAME_EN: &str = "zencode";
pub const NAME_KO: &str = "젠코드";
pub const NAME_JA: &str = "ゼンコード";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// "zencode  젠코드  ゼンコード"
pub fn full_name() -> String {
    format!("{NAME_EN}  {NAME_KO}  {NAME_JA}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::str_width;

    #[test]
    fn localized_names_are_full_width() {
        assert_eq!(str_width(NAME_KO), 6);
        assert_eq!(str_width(NAME_JA), 10);
        assert_eq!(str_width(&full_name()), 7 + 2 + 6 + 2 + 10);
    }
}
