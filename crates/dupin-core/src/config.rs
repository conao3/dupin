//! `config.toml`: user settings. Every key is optional; a missing file is the defaults.

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The global shortcut that shows and hides the panel, as Rune parses accelerators:
    /// modifiers (`cmd`, `alt`, `ctrl`, `shift`) and a key joined by `+`.
    pub hotkey: String,
    /// The most files a Spotlight search shows.
    pub file_limit: usize,
    /// Where the panel opens on the display under the pointer.
    pub position: Position,
}

/// Where the panel opens. Results always grow downwards from the field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    /// The field at the display's vertical center.
    #[default]
    Center,
    /// The field near the top, about where Spotlight puts it.
    Top,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey: "cmd+space".into(),
            file_limit: 200,
            position: Position::Center,
        }
    }
}

impl Config {
    pub fn parse(src: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(src)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_command_space() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
        assert_eq!(Config::default().hotkey, "cmd+space");
    }

    #[test]
    fn reads_the_hotkey() {
        assert_eq!(
            Config::parse("hotkey = \"alt+space\"").unwrap().hotkey,
            "alt+space"
        );
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(Config::parse("hotkye = \"alt+space\"").is_err());
    }

    #[test]
    fn reads_the_position() {
        assert_eq!(Config::default().position, Position::Center);
        assert_eq!(
            Config::parse("position = \"top\"").unwrap().position,
            Position::Top
        );
        assert!(Config::parse("position = \"middle\"").is_err());
    }
}
