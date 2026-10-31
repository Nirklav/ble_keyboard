//! User config file: [`Config`] as TOML.
//!
//! Files are written by hand-rolled code instead of `toml::to_string_pretty`, so that every
//! keyboard row is one line with aligned columns, which makes a layout readable as a grid:
//!
//! ```toml
//! [settings]
//! tapping_term_ms = 200
//! require_prior_idle_ms = 150
//! combo_term_ms = 50
//! ...
//!
//! [[layouts]]
//! name = "MAIN"
//! rows = [
//!     [{ Key = "Escape" }, { KeyWithMod = ["Num1", "LeftShift"] }, ...],
//!     ...
//! ]
//! joystick = { left = { MouseKey = "Left" }, stick = { MouseKey = "Middle" }, right = { MouseKey = "Right" } }
//! ```

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use protocol::config::{Config, Format, ValidationError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Doesn't validate the config.
pub fn load(path: &Path) -> Result<Config> {
    let text = fs::read_to_string(path).with_context(|| format!("can't read {}", path.display()))?;
    parse(&text).with_context(|| format!("can't parse {}", path.display()))
}

/// Doesn't validate the config. An older version is upgraded to [`CONFIG_VERSION`](protocol::config::CONFIG_VERSION), like [`Config::from_bytes`] does.
pub fn parse(text: &str) -> Result<Config> {
    #[derive(Deserialize)]
    struct Header {
        version: u16,
    }

    // toml errors already contain the line, column and the source snippet.
    let Header { version } = toml::from_str(text)?;
    Ok(Config::decode(version, &Toml(text))?)
}

/// TOML text of a config, see [`Config::decode`].
struct Toml<'a>(&'a str);

impl Format for Toml<'_> {
    type Error = toml::de::Error;

    fn decode<T: DeserializeOwned>(&self) -> Result<T, toml::de::Error> {
        toml::from_str(self.0)
    }
}

pub fn save(path: &Path, config: &Config) -> Result<()> {
    fs::write(path, format(config)?).with_context(|| format!("can't write {}", path.display()))
}

/// One keyboard row per line.
pub fn format(config: &Config) -> Result<String> {
    let mut out = String::new();
    writeln!(out, "version = {}", config.version)?;
    writeln!(out, "width = {}", config.width)?;
    writeln!(out, "height = {}", config.height)?;

    writeln!(out)?;
    writeln!(out, "[settings]")?;
    // One `name = value` line per field.
    write!(out, "{}", toml::to_string(&config.settings)?)?;

    for layout in &config.layouts {
        writeln!(out)?;
        writeln!(out, "[[layouts]]")?;
        writeln!(out, "name = {}", inline(&layout.name)?)?;

        let rows = layout
            .rows
            .iter()
            .map(|row| row.iter().map(inline).collect::<Result<Vec<_>>>())
            .collect::<Result<Vec<_>>>()?;
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> = (0..columns)
            .map(|x| rows.iter().filter_map(|row| row.get(x)).map(String::len).max().unwrap_or(0))
            .collect();

        writeln!(out, "rows = [")?;
        for row in &rows {
            let mut line = String::from("    [");
            for (x, cell) in row.iter().enumerate() {
                line.push_str(cell);
                if x + 1 < row.len() {
                    line.push_str(", ");
                    // Pad to the column width (+ the ", " written above).
                    line.extend(std::iter::repeat_n(' ', widths[x] - cell.len()));
                }
            }
            line.push_str("],");
            writeln!(out, "{line}")?;
        }
        writeln!(out, "]")?;
        writeln!(out, "joystick = {}", inline(&layout.joystick)?)?;
    }

    for combo in &config.combos {
        writeln!(out)?;
        writeln!(out, "[[combos]]")?;
        writeln!(out, "keys = {}", inline(&combo.keys)?)?;
        writeln!(out, "action = {}", inline(&combo.action)?)?;
    }

    Ok(out)
}

/// `value` as an inline TOML value, e.g. `{ Key = "A" }`.
fn inline<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    let mut out = String::new();
    value.serialize(toml::ser::ValueSerializer::new(&mut out))?;
    Ok(out)
}

/// [`ValidationError`] with the name of the layout it is about, if any.
pub fn describe(error: &ValidationError, config: &Config) -> String {
    let layout = match *error {
        ValidationError::NameTooLong { layout, .. }
        | ValidationError::NameNotAscii { layout }
        | ValidationError::Height { layout, .. }
        | ValidationError::Width { layout, .. }
        | ValidationError::MultipleFn { layout, .. }
        | ValidationError::FnLayoutIndex { layout, .. }
        | ValidationError::JoystickFn { layout }
        | ValidationError::Key { layout, .. }
        | ValidationError::Joystick { layout, .. } => Some(layout),
        _ => None,
    };
    match layout.and_then(|index| config.layouts.get(index)) {
        Some(layout) => format!("{error} (layout \"{}\")", layout.name),
        None => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use protocol::config::{JoystickButtons, Settings, CONFIG_VERSION};

    use super::*;
    use crate::EXAMPLE_CONFIG as EXAMPLE;

    #[test]
    fn example_is_the_factory_config() {
        assert_eq!(parse(EXAMPLE).unwrap(), protocol::board::factory_config());
    }

    #[test]
    fn format_roundtrip() {
        let config = parse(EXAMPLE).unwrap();
        let text = format(&config).unwrap();
        assert_eq!(parse(&text).unwrap(), config);
        // The example is kept in the format the app writes (after its header comment).
        let body: String = EXAMPLE.lines().skip_while(|l| l.starts_with('#')).map(|l| format!("{l}\n")).collect();
        assert_eq!(text, body.trim_start());
    }

    #[test]
    fn old_versions_are_upgraded() {
        // Versions 1 and 2 have no `[settings]`, version 1 no joystick buttons either.
        let (head, rest) = EXAMPLE.split_once("[settings]").unwrap();
        let (_, layouts) = rest.split_once("[[layouts]]").unwrap();
        let without_settings = format!("{head}[[layouts]]{layouts}");
        let version_2 = without_settings.replace("version = 4", "version = 2");
        let version_1 = without_settings.replace("version = 4", "version = 1").replace("joystick = ", "# joystick = ");
        let example = parse(EXAMPLE).unwrap();
        for (text, has_joystick) in [(version_1, false), (version_2, true)] {
            let config = parse(&text).unwrap();
            assert_eq!(config.version, CONFIG_VERSION);
            assert_eq!(config.settings, Settings::default());
            let mut expected = example.layouts.clone();
            if !has_joystick {
                expected.iter_mut().for_each(|layout| layout.joystick = JoystickButtons::default());
            }
            assert_eq!(config.layouts, expected);
            config.validate_for(10, 5).unwrap();
        }
    }

    #[test]
    fn version_3_is_upgraded() {
        let text = EXAMPLE.replace("version = 4", "version = 3").replace("tapping_term_ms = 200", "tapping_term_ms = 250");
        // Version 3 has only the tap-hold settings.
        let new_settings = ["combo_term_ms", "debounce_ms", "sleep_", "display_", "joystick_"];
        let text = text.lines().filter(|l| !new_settings.iter().any(|name| l.starts_with(name))).collect::<Vec<_>>().join("\n");
        let config = parse(&text).unwrap();
        assert_eq!(config.version, CONFIG_VERSION);
        assert_eq!(config.settings, Settings { tapping_term_ms: 250, ..Settings::default() });
        config.validate_for(10, 5).unwrap();
    }

    #[test]
    fn describe_names_layout() {
        let mut config = parse(EXAMPLE).unwrap();
        config.layouts[2].rows[3].pop();
        let error = config.validate().unwrap_err();
        assert_eq!(describe(&error, &config), "layout 2: row 3 has 9 keys, expected 10 (layout \"GAME\")");
    }
}
