//! Manifest text reader for the facts Cargo metadata does not expose.
//!
//! `cargo metadata` reports *resolved* values. It therefore cannot show whether a
//! member inherited a shared field from `[workspace.package]` or hard-coded the
//! same literal, and it does not expose the `[lints]` table at all. Those two
//! facts are part of the Phase 0 contract, so the checker reads them out of the
//! manifest text.
//!
//! The workspace has no TOML parser dependency, and Phase 0 adds none, so this
//! reads the subset of TOML the repository's own manifests use:
//!
//! - table headers, with arrays of tables kept distinct from same-named tables;
//! - dotted keys and single-line inline tables;
//! - booleans, and string values in basic (`"…"`) or literal (`'…'`) form;
//! - LF and CRLF line endings, and an optional leading byte-order mark.
//!
//! Comments are removed, and a `#` inside a string is not a comment. A multi-line
//! string is replaced by an empty basic string, so text that merely looks like an
//! assignment inside a value is never read as one, and the text on either side of
//! the string cannot fuse into an identifier the manifest does not contain.
//!
//! A spelling outside that subset reads as absent, which makes the checker fail
//! loudly with an actionable message rather than silently accept a manifest whose
//! policy is not actually in effect.
//!
//! Every function here is pure, so the rules that depend on manifest text can be
//! tested without a filesystem.

use std::collections::BTreeSet;

/// Table that declares the shared package fields members inherit.
pub const WORKSPACE_PACKAGE_TABLE: &str = "workspace.package";
/// Table that declares a member's own package fields.
pub const PACKAGE_TABLE: &str = "package";
/// Table that declares the lint policy opt-in.
pub const LINTS_TABLE: &str = "lints";
/// Table that declares the pinned toolchain in `rust-toolchain.toml`.
pub const TOOLCHAIN_TABLE: &str = "toolchain";

/// One `key = value` assignment, together with the table that owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Assignment {
    /// Dotted table path, empty at document scope.
    table: String,
    /// Key as written with whitespace removed, which may itself be dotted.
    key: String,
    /// Value as written, trimmed.
    value: String,
}

/// A Cargo manifest read as flat table, key, and value assignments.
///
/// The table a key belongs to is preserved, because the same key text means
/// different things in different tables: `lints.workspace = true` at document
/// scope enables workspace lint inheritance, while the identical text inside
/// `[package]` sets an unrelated `package.lints.workspace` key and leaves the
/// workspace lints disabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    assignments: Vec<Assignment>,
}

impl Manifest {
    /// Reads `text` as manifest assignments.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut assignments = Vec::new();
        let mut table = String::new();

        for line in code_lines(text) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(header) = table_header(line) {
                table = header;
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                // Continuation lines of a multi-line array carry no assignment.
                continue;
            };
            let key = compact(key);
            let value = value.trim();

            if let Some(inline) = value
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
            {
                // `version = { workspace = true }` is Cargo's other spelling of
                // `version.workspace = true`, so the inline form is normalized to
                // a dotted key and both are recognized by the same lookup.
                for entry in inline.split(',') {
                    if let Some((inner_key, inner_value)) = entry.split_once('=') {
                        assignments.push(Assignment {
                            table: table.clone(),
                            key: format!("{key}.{}", compact(inner_key)),
                            value: inner_value.trim().to_owned(),
                        });
                    }
                }
                continue;
            }

            assignments.push(Assignment {
                table: table.clone(),
                key,
                value: value.to_owned(),
            });
        }

        Self { assignments }
    }

    /// Returns the string assigned to `key` in `table`.
    ///
    /// A basic (`"…"`) or literal (`'…'`) string is returned without its
    /// delimiters. An empty string, a value with no matching delimiters, and an
    /// absent key all read as `None`, so a blank shared field cannot satisfy a
    /// presence requirement.
    #[must_use]
    pub fn string(&self, table: &str, key: &str) -> Option<&str> {
        let unquoted = unquote(self.value(table, key)?)?;
        (!unquoted.is_empty()).then_some(unquoted)
    }

    /// Reports whether `key` in `table` is assigned the boolean `true`.
    #[must_use]
    pub fn is_true(&self, table: &str, key: &str) -> bool {
        self.value(table, key) == Some("true")
    }

    /// Reports whether the manifest opts into the workspace lint policy.
    ///
    /// Cargo enables workspace lint inheritance only through the root `[lints]`
    /// table with `workspace = true`, or the equivalent document-scope
    /// `lints.workspace = true` written before any table header. The same dotted
    /// text inside `[package]`, or inside any other table, is a different key that
    /// leaves the workspace lints disabled, so it is not an opt-in.
    #[must_use]
    pub fn inherits_workspace_lints(&self) -> bool {
        self.is_true(LINTS_TABLE, "workspace") || self.is_true("", "lints.workspace")
    }

    /// Returns the `[package]` fields the manifest inherits from
    /// `[workspace.package]`.
    ///
    /// A field is inherited only when the member declares it explicitly, as
    /// `<field>.workspace = true` or `<field> = { workspace = true }`. A
    /// hard-coded literal is not inheritance even when it currently agrees with
    /// the workspace value.
    #[must_use]
    pub fn inherited_package_fields(&self) -> BTreeSet<String> {
        self.assignments
            .iter()
            .filter(|assignment| assignment.table == PACKAGE_TABLE && assignment.value == "true")
            .filter_map(|assignment| assignment.key.strip_suffix(".workspace"))
            .map(str::to_owned)
            .collect()
    }

    fn value(&self, table: &str, key: &str) -> Option<&str> {
        self.assignments
            .iter()
            .find(|assignment| assignment.table == table && assignment.key == key)
            .map(|assignment| assignment.value.as_str())
    }
}

/// Returns the table that following assignments belong to, when `line` is a table
/// header.
///
/// An array-of-tables header such as `[[bin]]` is reported under a bracketed name
/// that no plain header can produce, so a key under it is never attributed to the
/// same-named table and never to the previous one either. `[[lints]]` is therefore
/// not the lint opt-in; Cargo rejects such a manifest outright, but the reader must
/// not depend on that.
fn table_header(line: &str) -> Option<String> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    match inner
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        Some(array) => Some(format!("[{}]", compact(array))),
        None => Some(compact(inner)),
    }
}

/// Removes a matching pair of basic (`"`) or literal (`'`) string delimiters.
fn unquote(value: &str) -> Option<&str> {
    ['"', '\''].into_iter().find_map(|delimiter| {
        value
            .strip_prefix(delimiter)
            .and_then(|rest| rest.strip_suffix(delimiter))
    })
}

/// Stands in for a discarded multi-line string.
///
/// An empty basic string keeps the value unreadable, because [`Manifest::string`]
/// rejects an empty string, while its delimiters stop the text on either side from
/// fusing into one identifier. A space would not be enough: [`compact`] removes
/// whitespace from keys, so `work"""x"""space` would still read as `workspace`.
const DISCARDED_STRING: &str = "\"\"";

/// Reduces manifest text to one code-only logical line per source line.
///
/// Comments are removed, a `#` inside a string is kept, and a multi-line string is
/// replaced by [`DISCARDED_STRING`] so its contents can never be read as
/// assignments. A leading byte-order mark is stripped, and `str::lines` accepts both
/// LF and CRLF.
fn code_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    // A multi-line string that is still open at the end of a source line, so its
    // continuation lines are discarded rather than parsed.
    let mut open: Option<char> = None;

    for line in text.trim_start_matches('\u{feff}').lines() {
        let characters: Vec<char> = line.chars().collect();
        let mut code = String::new();
        let mut index = 0;

        while index < characters.len() {
            if let Some(delimiter) = open {
                code.push_str(DISCARDED_STRING);
                match closing_triple(&characters, index, delimiter) {
                    Some(end) => {
                        index = end;
                        open = None;
                    }
                    None => index = characters.len(),
                }
                continue;
            }

            let character = characters[index];
            if character == '#' {
                break;
            }
            if character != '"' && character != '\'' {
                code.push(character);
                index += 1;
                continue;
            }

            if is_triple(&characters, index, character) {
                code.push_str(DISCARDED_STRING);
                index += 3;
                match closing_triple(&characters, index, character) {
                    Some(end) => index = end,
                    None => {
                        open = Some(character);
                        index = characters.len();
                    }
                }
                continue;
            }

            let (string, next) = single_line_string(&characters, index, character);
            code.push_str(&string);
            index = next;
        }

        lines.push(code);
    }

    lines
}

/// Reports whether a triple delimiter starts at `index`.
fn is_triple(characters: &[char], index: usize, delimiter: char) -> bool {
    characters.get(index) == Some(&delimiter)
        && characters.get(index + 1) == Some(&delimiter)
        && characters.get(index + 2) == Some(&delimiter)
}

/// Returns the index just past the first triple delimiter at or after `index`.
fn closing_triple(characters: &[char], index: usize, delimiter: char) -> Option<usize> {
    (index..characters.len())
        .find(|position| is_triple(characters, *position, delimiter))
        .map(|position| position + 3)
}

/// Copies a single-line string verbatim, returning it and the index just past it.
///
/// A basic string escapes the following character with `\`; a literal string has no
/// escapes. An unterminated string ends with the line, which keeps one malformed
/// line from consuming the rest of the manifest.
fn single_line_string(characters: &[char], index: usize, delimiter: char) -> (String, usize) {
    let mut string = String::from(delimiter);
    let mut position = index + 1;

    while position < characters.len() {
        let character = characters[position];
        string.push(character);
        position += 1;

        if character == '\\' && delimiter == '"' {
            if let Some(escaped) = characters.get(position) {
                string.push(*escaped);
                position += 1;
            }
            continue;
        }
        if character == delimiter {
            break;
        }
    }

    (string, position)
}

/// Removes every whitespace character, so key spelling does not depend on layout.
fn compact(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}
