//! User config file: [`Config`] as TOML.
//!
//! Files are written by hand-rolled code instead of `toml::to_string_pretty`, so that every
//! keyboard row is one line with aligned columns, which makes a layout readable as a grid:
//!
//! ```toml
//! [[layouts]]
//! name = "MAIN"
//! rows = [
//!     [{ Key = "Escape" }, { KeyWithMod = ["Num1", "LeftShift"] }, ...],
//!     ...
//! ]
//! ```

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use protocol::config::{Config, ValidationError};
use serde::Serialize;

/// Reads and parses a TOML config, doesn't validate it.
pub fn load(path: &Path) -> Result<Config> {
    let text = fs::read_to_string(path).with_context(|| format!("can't read {}", path.display()))?;
    parse(&text).with_context(|| format!("can't parse {}", path.display()))
}

/// Parses a TOML config, doesn't validate it.
pub fn parse(text: &str) -> Result<Config> {
    // toml errors already contain the line, column and the source snippet.
    Ok(toml::from_str(text)?)
}

/// Writes `config` as TOML.
pub fn save(path: &Path, config: &Config) -> Result<()> {
    fs::write(path, format(config)?).with_context(|| format!("can't write {}", path.display()))
}

/// `config` as TOML, one keyboard row per line.
pub fn format(config: &Config) -> Result<String> {
    let mut out = String::new();
    writeln!(out, "version = {}", config.version)?;
    writeln!(out, "width = {}", config.width)?;
    writeln!(out, "height = {}", config.height)?;

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
        | ValidationError::FnLayoutIndex { layout, .. } => Some(layout),
        _ => None,
    };
    match layout.and_then(|index| config.layouts.get(index)) {
        Some(layout) => format!("{error} (layout \"{}\")", layout.name),
        None => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../examples/config.toml");

    #[test]
    fn example_is_valid() {
        parse(EXAMPLE).unwrap().validate_for(10, 5).unwrap();
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
    fn describe_names_layout() {
        let mut config = parse(EXAMPLE).unwrap();
        config.layouts[2].rows[3].pop();
        let error = config.validate().unwrap_err();
        assert_eq!(describe(&error, &config), "layout 2: row 3 has 9 keys, expected 10 (layout \"GAME\")");
    }
}
