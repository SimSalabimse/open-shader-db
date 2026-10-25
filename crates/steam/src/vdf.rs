// SPDX-License-Identifier: MIT

//! Small from-scratch Valve KeyValues (VDF) text parser.
//!
//! Handles quoted strings, escapes, nested blocks, `//` and `/* */` comments.
//! Rejects hostile input: 1 MiB file cap, depth 32, token cap.
//!
//! Duplicate keys in a table keep the **last** value.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::Error;

/// Maximum VDF file size (1 MiB).
pub const MAX_VDF_BYTES: u64 = 1024 * 1024;
/// Maximum `{` nesting.
pub const MAX_VDF_DEPTH: u32 = 32;
/// Maximum keys, values, and braces counted as tokens.
pub const MAX_VDF_TOKENS: u32 = 65_536;

/// A parsed VDF document with a single root key.
#[derive(Clone, PartialEq, Eq)]
pub struct VdfDocument {
    pub root_key: String,
    pub root: VdfValue,
}

impl fmt::Debug for VdfDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VdfDocument")
            .field("root_key", &self.root_key)
            .field("root", &self.root)
            .finish()
    }
}

/// A VDF string or nested table.
///
/// [`Debug`] redacts string values because they may hold library paths.
#[derive(Clone, PartialEq, Eq)]
pub enum VdfValue {
    String(String),
    Table(BTreeMap<String, VdfValue>),
}

impl fmt::Debug for VdfValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(_) => f.write_str("String(<redacted>)"),
            Self::Table(table) => f.debug_tuple("Table").field(table).finish(),
        }
    }
}

impl VdfValue {
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            Self::Table(_) => None,
        }
    }

    #[must_use]
    pub fn as_table(&self) -> Option<&BTreeMap<String, VdfValue>> {
        match self {
            Self::Table(t) => Some(t),
            Self::String(_) => None,
        }
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&VdfValue> {
        self.as_table()?.get(key)
    }
}

/// Read a regular file (never a FIFO) and parse it as VDF text.
pub fn parse_vdf_file(path: &Path) -> Result<VdfDocument, Error> {
    let bytes = read_regular_file(path, MAX_VDF_BYTES)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| Error::InvalidUtf8)?;
    parse_vdf(text)
}

/// Parse VDF text with the default hostility caps.
pub fn parse_vdf(text: &str) -> Result<VdfDocument, Error> {
    parse_vdf_with_limits(text, MAX_VDF_DEPTH, MAX_VDF_TOKENS)
}

pub(crate) fn parse_vdf_with_limits(
    text: &str,
    max_depth: u32,
    max_tokens: u32,
) -> Result<VdfDocument, Error> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut p = Parser {
        src: text,
        pos: 0,
        tokens: 0,
        max_tokens,
        max_depth,
    };
    p.skip_ws_and_comments()?;
    let root_key = p.parse_string()?;
    let root = p.parse_value(0)?;
    p.skip_ws_and_comments()?;
    if !p.eof() {
        return Err(Error::Syntax);
    }
    Ok(VdfDocument { root_key, root })
}

pub(crate) fn read_regular_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>, Error> {
    let file = open_read_file(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() {
        return Err(Error::NotARegularFile);
    }
    if opened.len() > max_bytes {
        return Err(Error::FileTooLarge {
            max: max_bytes,
            got: opened.len(),
        });
    }
    let mut buf = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut buf)?;
    if buf.len() as u64 > max_bytes {
        return Err(Error::FileTooLarge {
            max: max_bytes,
            got: buf.len() as u64,
        });
    }
    Ok(buf)
}

/// Open for read. On Unix uses `O_NONBLOCK` so a FIFO (or a swap to a FIFO
/// after a path check) does not block the thread. File type is taken from the
/// **opened** handle.
fn open_read_file(path: &Path) -> Result<File, Error> {
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)?;
        if !file.metadata()?.is_file() {
            return Err(Error::NotARegularFile);
        }
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        let meta = std::fs::metadata(path)?;
        if !meta.is_file() {
            return Err(Error::NotARegularFile);
        }
        let file = File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err(Error::NotARegularFile);
        }
        Ok(file)
    }
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
    tokens: u32,
    max_tokens: u32,
    max_depth: u32,
}

impl Parser<'_> {
    fn eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn rest(&self) -> &str {
        self.src.get(self.pos..).unwrap_or("")
    }

    fn bump(&mut self) -> Option<char> {
        let mut chars = self.rest().chars();
        let ch = chars.next()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn charge(&mut self) -> Result<(), Error> {
        self.tokens = self.tokens.saturating_add(1);
        if self.tokens > self.max_tokens {
            return Err(Error::TooManyTokens {
                max: self.max_tokens,
                got: self.tokens,
            });
        }
        Ok(())
    }

    fn skip_ws_and_comments(&mut self) -> Result<(), Error> {
        loop {
            while self.peek().is_some_and(char::is_whitespace) {
                self.bump();
            }
            let rest = self.rest();
            if rest.starts_with("//") {
                self.pos += 2;
                while let Some(ch) = self.peek() {
                    self.bump();
                    if ch == '\n' {
                        break;
                    }
                }
            } else if rest.starts_with("/*") {
                self.pos += 2;
                let mut closed = false;
                while !self.eof() {
                    if self.rest().starts_with("*/") {
                        self.pos += 2;
                        closed = true;
                        break;
                    }
                    self.bump();
                }
                if !closed {
                    return Err(Error::UnclosedComment);
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    fn parse_string(&mut self) -> Result<String, Error> {
        self.skip_ws_and_comments()?;
        self.charge()?;
        match self.peek() {
            Some('"') => self.parse_quoted(),
            Some(ch) if is_bare_start(ch) => self.parse_bare(),
            _ => Err(Error::Syntax),
        }
    }

    fn parse_quoted(&mut self) -> Result<String, Error> {
        if self.bump() != Some('"') {
            return Err(Error::Syntax);
        }
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(Error::UnclosedString),
                Some('"') => return Ok(out),
                Some('\\') => match self.bump() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some(_) | None => return Err(Error::InvalidEscape),
                },
                Some(ch) => out.push(ch),
            }
        }
    }

    fn parse_bare(&mut self) -> Result<String, Error> {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() || matches!(ch, '{' | '}' | '"') {
                break;
            }
            self.bump();
        }
        let s = self.src.get(start..self.pos).ok_or(Error::Syntax)?;
        if s.is_empty() {
            return Err(Error::Syntax);
        }
        Ok(s.to_string())
    }

    fn parse_value(&mut self, depth: u32) -> Result<VdfValue, Error> {
        self.skip_ws_and_comments()?;
        if self.peek() == Some('{') {
            self.parse_table(depth)
        } else {
            Ok(VdfValue::String(self.parse_string()?))
        }
    }

    fn parse_table(&mut self, depth: u32) -> Result<VdfValue, Error> {
        if depth >= self.max_depth {
            return Err(Error::TooDeep {
                max: self.max_depth,
                got: depth.saturating_add(1),
            });
        }
        self.skip_ws_and_comments()?;
        if self.bump() != Some('{') {
            return Err(Error::Syntax);
        }
        self.charge()?;
        let mut table = BTreeMap::new();
        loop {
            self.skip_ws_and_comments()?;
            if self.peek() == Some('}') {
                self.bump();
                self.charge()?;
                return Ok(VdfValue::Table(table));
            }
            if self.eof() {
                return Err(Error::UnclosedBlock);
            }
            let key = self.parse_string()?;
            let value = self.parse_value(depth.saturating_add(1))?;
            // Last value wins when the same key appears more than once.
            table.insert(key, value);
        }
    }
}

fn is_bare_start(ch: char) -> bool {
    !ch.is_whitespace() && !matches!(ch, '{' | '}' | '"')
}
