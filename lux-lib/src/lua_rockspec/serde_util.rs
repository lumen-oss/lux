use std::fmt::Display;

use itertools::Itertools;
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer,
};

/// A visitor and [`de::DeserializeSeed`] that collects a raw `serde_value::Value`
/// from piccolo's deserializer, converting byte strings to Rust strings and
/// preserving integer map keys (which piccolo emits for Lua sequences).
pub(crate) struct LuaValueSeed;

impl<'de> Visitor<'de> for LuaValueSeed {
    type Value = serde_value::Value;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("any Lua value")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
        Ok(serde_value::Value::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
        Ok(serde_value::Value::I64(v))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
        Ok(serde_value::Value::U64(v))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
        Ok(serde_value::Value::F64(v))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        Ok(serde_value::Value::String(v.to_string()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
        Ok(serde_value::Value::String(v))
    }

    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
        let s = std::str::from_utf8(v).map_err(de::Error::custom)?;
        Ok(serde_value::Value::String(s.to_string()))
    }

    fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
        self.visit_bytes(&v)
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(serde_value::Value::Unit)
    }

    fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
        d.deserialize_any(LuaValueSeed)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(serde_value::Value::Unit)
    }

    fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut arr = Vec::new();
        while let Some(v) = seq.next_element_seed(LuaValueSeed)? {
            arr.push(v);
        }
        Ok(serde_value::Value::Seq(arr))
    }

    fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut obj = std::collections::BTreeMap::new();
        while let Some(key) = map.next_key_seed(LuaValueSeed)? {
            let val = map.next_value_seed(LuaValueSeed)?;
            obj.insert(key, val);
        }
        Ok(serde_value::Value::Map(obj))
    }
}

impl<'de> de::DeserializeSeed<'de> for LuaValueSeed {
    type Value = serde_value::Value;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
}

/// Normalise a `serde_value::Value` that came from piccolo (our Lua runtime).
///
/// Piccolo represents Lua sequences-with-holes (e.g. `{nil, nil, "foo"}`) as a
/// `Value::Map` with integer keys rather than a `Value::Seq`. This function
/// detects that case and converts such a map into a `Value::Seq` sorted by
/// index, leaving all other values untouched.
pub(crate) fn normalize_lua_value(value: serde_value::Value) -> serde_value::Value {
    match value {
        // piccolo_util serializes Lua strings as Bytes; convert to String
        serde_value::Value::Bytes(bytes) => match String::from_utf8(bytes.clone()) {
            Ok(s) => serde_value::Value::String(s),
            Err(_) => serde_value::Value::Bytes(bytes),
        },
        serde_value::Value::Map(map)
            if map
                .keys()
                .all(|k| matches!(k, serde_value::Value::I64(_) | serde_value::Value::U64(_))) =>
        {
            let seq = map
                .iter()
                .sorted_by_key(|(k, _)| match k {
                    serde_value::Value::I64(i) => *i,
                    serde_value::Value::U64(u) => *u as i64,
                    _ => unreachable!(),
                })
                .map(|(_, v)| normalize_lua_value(v.clone()))
                .collect();
            serde_value::Value::Seq(seq)
        }
        serde_value::Value::Map(map) => serde_value::Value::Map(
            map.into_iter()
                .map(|(k, v)| (normalize_lua_value(k), normalize_lua_value(v)))
                .collect(),
        ),
        serde_value::Value::Seq(seq) => {
            serde_value::Value::Seq(seq.into_iter().map(normalize_lua_value).collect())
        }
        other => other,
    }
}

#[derive(Hash, Debug, Eq, PartialEq, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum LuaTableKey {
    IntKey(u64),
    StringKey(String),
}

/// Deserialize a json value into a Vec<T>, treating empty json objects as empty lists
/// If the json value is a string, this returns a singleton vector containing that value.
/// This is needed to be able to deserialise RockSpec tables that luarocks
/// also allows to be strings.
pub(crate) fn deserialize_vec_from_lua_array_or_string<'de, D, T>(
    deserializer: D,
) -> std::result::Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: From<String>,
    T: Deserialize<'de>,
{
    let value = normalize_lua_value(serde_value::Value::deserialize(deserializer)?);
    if let serde_value::Value::String(str) = value {
        Ok(vec![T::from(str)])
    } else {
        let value = normalize_lua_value(value);
        value.clone().deserialize_into().map_err(|err| {
            de::Error::custom(format!(
                "expected a string or a list of strings, but got: {value:?} ({err})"
            ))
        })
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
            DisplayLuaValue::String(s) => write!(buf, "\"{s}\"")?,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_lua_value() {
        let value = DisplayLuaValue::String("hello".to_string());
        assert_eq!(format!("{value}"), "\"hello\"");

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
