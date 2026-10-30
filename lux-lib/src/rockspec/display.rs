use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::path::PathBuf;

use itertools::Itertools;
use path_slash::PathBufExt as _;
use url::Url;

struct StringAnalysis {
    /// The lowest number of equal signs needed for long-string delimeters
    /// around this string.
    long_string_equal_signs: usize,
    has_newline: bool,
    has_nonprintable: bool,
}

impl From<&str> for StringAnalysis {
    /// Analyze the string in a single pass.
    fn from(value: &str) -> Self {
        // The number of consecutive equal signs immediately between all pairs
        // of ] characters, including at the end of a long-delimiter string.
        let mut equal_signs = HashSet::new();
        let mut has_newline = false;
        let mut has_nonprintable = false;
        let bytes = value.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b']' => {
                    i += 1;
                    let bytes_after_brace = &bytes[i..];
                    let non_equal_sign = bytes_after_brace
                        .iter()
                        .copied()
                        .enumerate()
                        .find(|&(_, c)| c != b'=');
                    match non_equal_sign {
                        None => {
                            // Still need to worry about `]` or `]=...` at the
                            // end of the string.
                            equal_signs.insert(bytes_after_brace.len());
                            break;
                        }
                        Some((index, b']')) => {
                            equal_signs.insert(index);

                            // We still want the `]` to be processed on the next
                            // loop, because:
                            // * `]==]===]` should push both a 2 and a 3 into
                            //   the equal signs hash.
                            // * `]==]` at the end of the string should push
                            //   both a 2 and a 0, because `[[...]==]]]` is an
                            //   invalid string literal.
                            i += index;
                        }
                        Some((index, _)) => {
                            // These are inoffensive equal signs, because they
                            // are not followed by `]` or the end of the string.
                            i += index;
                        }
                    }
                }
                b'\n' | b'\r' => {
                    has_newline = true;
                    // Lua considers end-of-line to be CR, LF, CR+LF or LF+CR.
                    // We consider any run of CRs without any LFs to be
                    // non-printable, because that can cause characters to be
                    // overwritten when output to a terminal. Any number of CRs
                    // paired with at least 1 LF are fine.
                    let bytes_from_newline = &bytes[i..];
                    let (lf, cr) = bytes_from_newline
                        .iter()
                        .copied()
                        .take_while(|&c| c == b'\n' || c == b'\r')
                        .fold((0usize, 0usize), |(lf, cr), c| {
                            if c == b'\n' {
                                (lf + 1, cr)
                            } else {
                                (lf, cr + 1)
                            }
                        });
                    i += lf + cr;
                    if lf == 0 {
                        has_nonprintable = true;
                    }
                }
                // Remaining printable bytes.
                b' '..=b'~' | b'\t' => {
                    i += 1;
                }
                _ => {
                    has_nonprintable = true;
                    i += 1;
                }
            }
        }

        #[allow(clippy::unwrap_used)]
        let long_string_equal_signs = (0..).find(move |i| !equal_signs.contains(i)).unwrap();

        Self {
            long_string_equal_signs,
            has_newline,
            has_nonprintable,
        }
    }
}
pub(crate) enum DisplayLuaValue {
    // NOTE(vhyrro): these are not used in the current implementation
    // Nil,
    // Number(f64),
    Boolean(bool),
    String(String),
    List(Vec<Self>),
    Table(Vec<DisplayLuaKV>),
}

pub(crate) struct DisplayLuaKV {
    pub(crate) key: String,
    pub(crate) value: DisplayLuaValue,
}

impl Display for DisplayLuaValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Write;
        let mut buf = String::new();
        match self {
            //DisplayLuaValue::Nil => write!(f, "nil"),
            //DisplayLuaValue::Number(n) => write!(f, "{n}"),
            DisplayLuaValue::Boolean(b) => write!(buf, "{b}")?,
            DisplayLuaValue::String(s) => {
                let analysis = StringAnalysis::from(s.as_str());
                if analysis.has_newline && !analysis.has_nonprintable {
                    // A long-delimiter string will look better for strings that
                    // are all printable with line breaks.
                    buf.push('[');
                    buf.extend(std::iter::repeat_n('=', analysis.long_string_equal_signs));
                    buf.push_str("[\n");
                    buf.push_str(s);
                    buf.push(']');
                    buf.extend(std::iter::repeat_n('=', analysis.long_string_equal_signs));
                    buf.push(']');
                } else {
                    // Escape all the string bytes.
                    // We do bytes instead of unicode characters because Lua strings
                    // only got unicode escapes as of version 5.3.
                    buf.push('"');
                    for c in s.bytes() {
                        match c {
                            b'"' => buf.push_str("\\\""),
                            b'\x07' => buf.push_str("\\a"),
                            b'\x08' => buf.push_str("\\b"),
                            b'\x0B' => buf.push_str("\\v"),
                            b'\x0C' => buf.push_str("\\f"),
                            b'\n' => buf.push_str("\\n"),
                            b'\r' => buf.push_str("\\r"),
                            b'\t' => buf.push_str("\\t"),
                            b'\\' => buf.push_str("\\\\"),
                            // Remaining ascii printables.
                            b' '..=b'~' => {
                                buf.push(c as char);
                            }
                            _ => {
                                // \ddd decimal escapes.
                                write!(buf, "\\{c:03}")?;
                            }
                        }
                    }
                    buf.push('"');
                }
            }
            DisplayLuaValue::List(l) => {
                writeln!(buf, "{{")?;
                for item in l {
                    writeln!(buf, "{item},")?;
                }
                write!(buf, "}}")?;
            }
            DisplayLuaValue::Table(t) => {
                writeln!(buf, "{{")?;

                for item in t {
                    writeln!(buf, "{item},")?;
                }

                write!(buf, "}}")?;
            }
        };
        let output = match stylua_lib::format_code(
            &buf,
            stylua_lib::Config::default(),
            None,
            stylua_lib::OutputVerification::Full,
        ) {
            Ok(formatted_code) => formatted_code,
            Err(_) => buf,
        };
        write!(f, "{output}")
    }
}

impl Display for DisplayLuaKV {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self
            .key
            .chars()
            .all(|c| c == '_' || c.is_ascii_alphanumeric())
        {
            write!(f, "['{}'] = {}", self.key, self.value)
        } else {
            write!(f, "{} = {}", self.key, self.value)
        }
    }
}

/// Trait for serializing a Lua structure from a rockspec into a `key = value` pair.
pub(crate) trait DisplayAsLuaKV {
    fn display_lua(&self) -> DisplayLuaKV;
}

pub(crate) trait DisplayAsLuaValue {
    fn display_lua_value(&self) -> DisplayLuaValue;
}

impl DisplayAsLuaValue for String {
    fn display_lua_value(&self) -> DisplayLuaValue {
        DisplayLuaValue::String(self.clone())
    }
}

impl DisplayAsLuaValue for bool {
    fn display_lua_value(&self) -> DisplayLuaValue {
        DisplayLuaValue::Boolean(*self)
    }
}

impl DisplayAsLuaValue for PathBuf {
    fn display_lua_value(&self) -> DisplayLuaValue {
        DisplayLuaValue::String(self.to_slash_lossy().into_owned())
    }
}

impl DisplayAsLuaValue for Vec<String> {
    fn display_lua_value(&self) -> DisplayLuaValue {
        DisplayLuaValue::List(self.iter().cloned().map(DisplayLuaValue::String).collect())
    }
}

impl DisplayAsLuaValue for Vec<PathBuf> {
    fn display_lua_value(&self) -> DisplayLuaValue {
        DisplayLuaValue::List(
            self.iter()
                .map(|p| DisplayLuaValue::String(p.to_slash_lossy().into_owned()))
                .collect(),
        )
    }
}

impl DisplayAsLuaValue for HashMap<String, String> {
    fn display_lua_value(&self) -> DisplayLuaValue {
        DisplayLuaValue::Table(
            self.iter()
                .map(|(k, v)| DisplayLuaKV {
                    key: k.clone(),
                    value: DisplayLuaValue::String(v.clone()),
                })
                .collect_vec(),
        )
    }
}

impl DisplayAsLuaValue for HashMap<String, PathBuf> {
    fn display_lua_value(&self) -> DisplayLuaValue {
        DisplayLuaValue::Table(
            self.iter()
                .map(|(k, v)| DisplayLuaKV {
                    key: k.clone(),
                    value: DisplayLuaValue::String(v.to_slash_lossy().into_owned()),
                })
                .collect_vec(),
        )
    }
}

impl DisplayAsLuaValue for Url {
    fn display_lua_value(&self) -> DisplayLuaValue {
        DisplayLuaValue::String(self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_lua_value() {
        let value = DisplayLuaValue::String("hello".to_string());
        assert_eq!(format!("{value}"), r#""hello""#);

        let value = DisplayLuaValue::String("he\"llo".to_string());
        assert_eq!(format!("{value}"), r#""he\"llo""#);

        let value = DisplayLuaValue::String("q\"a'".to_string());
        assert_eq!(format!("{value}"), r#""q\"a'""#);

        let value = DisplayLuaValue::String(
            "1\"2\x073\x084\x0B5\x0C6\n7\r8\t9'a\\b\u{FFFFF}c\0d\u{1}e".to_string(),
        );
        assert_eq!(
            format!("{value}"),
            r#""1\"2\a3\b4\v5\f6\n7\r8\t9'a\\b\243\191\191\191c\000d\001e""#
        );
        let value = DisplayLuaValue::String("\n".to_string());
        assert_eq!(format!("{value}"), "[[\n\n]]");
        let value = DisplayLuaValue::String("\n]".to_string());
        assert_eq!(format!("{value}"), "[=[\n\n]]=]");
        let value = DisplayLuaValue::String("first line\nsecond line".to_string());
        assert_eq!(format!("{value}"), "[[\nfirst line\nsecond line]]");

        let value = DisplayLuaValue::String("first line\nsecond line]".to_string());
        assert_eq!(format!("{value}"), "[=[\nfirst line\nsecond line]]=]");

        let value = DisplayLuaValue::String("first line\nsecond line]=]".to_string());
        assert_eq!(format!("{value}"), "[==[\nfirst line\nsecond line]=]]==]");

        let value = DisplayLuaValue::String("first line\nsecond line]\nthird line".to_string());
        assert_eq!(
            format!("{value}"),
            "[[\nfirst line\nsecond line]\nthird line]]"
        );

        let value = DisplayLuaValue::String("first line\nsecond line]]\nthird line".to_string());
        assert_eq!(
            format!("{value}"),
            "[=[\nfirst line\nsecond line]]\nthird line]=]"
        );

        let value = DisplayLuaValue::String("first line\nsecond line]=]\nthird line".to_string());
        assert_eq!(
            format!("{value}"),
            "[[\nfirst line\nsecond line]=]\nthird line]]"
        );

        let value = DisplayLuaValue::String("first line\nsecond line]=]]\nthird line".to_string());
        assert_eq!(
            format!("{value}"),
            "[==[\nfirst line\nsecond line]=]]\nthird line]==]"
        );

        let value = DisplayLuaValue::String("first line\nsecond line]]=]\nthird line".to_string());
        assert_eq!(
            format!("{value}"),
            "[==[\nfirst line\nsecond line]]=]\nthird line]==]"
        );

        let value = DisplayLuaValue::String("\tfirst line\n\tsecond line".to_string());
        assert_eq!(format!("{value}"), "[[\n\tfirst line\n\tsecond line]]");

        let value = DisplayLuaValue::String("\tfirst line\r\n\tsecond line".to_string());
        assert_eq!(format!("{value}"), "[[\n\tfirst line\r\n\tsecond line]]");

        let value = DisplayLuaValue::String("\tfirst line\r\tsecond line".to_string());
        assert_eq!(format!("{value}"), r#""\tfirst line\r\tsecond line""#);

        let value = DisplayLuaValue::Boolean(true);
        assert_eq!(format!("{value}"), "true");

        let value = DisplayLuaValue::List(vec![
            DisplayLuaValue::String("hello".to_string()),
            DisplayLuaValue::Boolean(true),
        ]);
        assert_eq!(format!("{value}"), "{\n\"hello\",\ntrue,\n}");

        let value = DisplayLuaValue::Table(vec![
            DisplayLuaKV {
                key: "key".to_string(),
                value: DisplayLuaValue::String("value".to_string()),
            },
            DisplayLuaKV {
                key: "key2".to_string(),
                value: DisplayLuaValue::Boolean(true),
            },
            DisplayLuaKV {
                key: "key3.key4".to_string(),
                value: DisplayLuaValue::Boolean(true),
            },
        ]);
        assert_eq!(
            format!("{value}"),
            "{\nkey = \"value\",\nkey2 = true,\n['key3.key4'] = true,\n}"
        );
    }
}
