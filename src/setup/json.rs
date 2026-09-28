//! A JSON settings file edited in place: objects keep their key order, and
//! the text keeps its indentation, line ends, byte-order mark and final
//! newline, so adding JevGate's hooks to someone's settings changes only
//! those lines. `serde_json`'s own map sorts keys, and its `preserve_order`
//! feature would also reorder the request bodies whose hashes key the answer
//! cache, asking every cached question again. Numbers and strings JevGate
//! does not change keep their text (`1e3`, `1.50`, a 30-digit integer,
//! `"\/"`): read as values, they came back as `1000.0`, `1.5`,
//! `1.2345678901234568e+29` and `"/"`, and stayed so after `--remove`.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize, de, ser::SerializeMap};
use serde_json::value::RawValue;
use std::fmt;

/// A JSON value whose objects keep their keys in the order they were read.
#[derive(Clone, Debug)]
pub(super) enum Json {
    Null,
    Bool(bool),
    /// A number, as its text.
    Number(Box<RawValue>),
    /// A string's value, and its text when it was read from a file.
    String(String, Option<Box<RawValue>>),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl PartialEq for Json {
    /// Numbers are equal when written alike, strings when their values are.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Number(a), Self::Number(b)) => a.get() == b.get(),
            (Self::String(a, _), Self::String(b, _)) => a == b,
            (Self::Array(a), Self::Array(b)) => a == b,
            (Self::Object(a), Self::Object(b)) => a == b,
            _ => false,
        }
    }
}

impl Json {
    /// An object of `entries`, in their order.
    pub fn object<'a>(entries: impl IntoIterator<Item = (&'a str, Json)>) -> Self {
        Self::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Json> {
        match self {
            Self::Object(entries) => entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Set `key` in an object: in its place when present, else at the end.
    pub fn set(&mut self, key: &str, value: Json) {
        if let Some(slot) = self.get_mut(key) {
            *slot = value;
        } else if let Self::Object(entries) = self {
            entries.push((key.to_string(), value));
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<Json> {
        let Self::Object(entries) = self else {
            return None;
        };
        let at = entries.iter().position(|(k, _)| k == key)?;
        Some(entries.remove(at).1)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text, _) => Some(text),
            _ => None,
        }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Json>> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// Whether this is an object with no keys, or an empty list.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Object(entries) => entries.is_empty(),
            Self::Array(items) => items.is_empty(),
            _ => false,
        }
    }
}

impl From<&str> for Json {
    fn from(text: &str) -> Self {
        Self::String(text.to_string(), None)
    }
}

impl From<u64> for Json {
    fn from(number: u64) -> Self {
        Self::Number(RawValue::from_string(number.to_string()).expect("a number is JSON"))
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        Self::from_raw(raw).map_err(de::Error::custom)
    }
}

impl Json {
    /// The value `raw` holds, whose numbers and strings keep their text.
    fn from_raw(raw: Box<RawValue>) -> serde_json::Result<Self> {
        let text = raw.get();
        Ok(match text.as_bytes().first() {
            Some(b'{') => Self::Object(
                serde_json::from_str::<Entries>(text)?
                    .0
                    .into_iter()
                    .map(|(key, value)| Ok((key, Self::from_raw(value)?)))
                    .collect::<serde_json::Result<_>>()?,
            ),
            Some(b'[') => Self::Array(
                serde_json::from_str::<Vec<Box<RawValue>>>(text)?
                    .into_iter()
                    .map(Self::from_raw)
                    .collect::<serde_json::Result<_>>()?,
            ),
            Some(b'"') => Self::String(serde_json::from_str(text)?, Some(raw)),
            Some(b't' | b'f') => Self::Bool(serde_json::from_str(text)?),
            Some(b'n') => Self::Null,
            _ => Self::Number(raw),
        })
    }
}

/// An object's entries in the order they were read, values unread.
struct Entries(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for Entries {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(EntriesVisitor)
    }
}

struct EntriesVisitor;

impl<'de> de::Visitor<'de> for EntriesVisitor {
    type Value = Entries;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Entries, A::Error> {
        let mut entries = Vec::new();
        while let Some(entry) = map.next_entry::<String, Box<RawValue>>()? {
            entries.push(entry);
        }
        Ok(Entries(entries))
    }
}

impl Serialize for Json {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Number(raw) | Self::String(_, Some(raw)) => raw.serialize(serializer),
            Self::String(text, None) => serializer.serialize_str(text),
            Self::Array(items) => items.serialize(serializer),
            Self::Object(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

/// How a file's text is laid out, so an edited document is written the same way.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Layout {
    indent: String,
    crlf: bool,
    bom: bool,
    final_newline: bool,
}

impl Default for Layout {
    /// A new file: two spaces, `\n`, a final newline.
    fn default() -> Self {
        Self {
            indent: "  ".into(),
            crlf: false,
            bom: false,
            final_newline: true,
        }
    }
}

/// The object a settings file holds, and how its text is laid out. Empty
/// text is an empty object; anything else must be one JSON object, since
/// JevGate never rewrites a file it cannot read whole (a comment included).
pub(super) fn parse(text: &str) -> Result<(Json, Layout)> {
    let bom = text.starts_with('\u{feff}');
    let body = text.trim_start_matches('\u{feff}');
    let layout = Layout {
        indent: indent(body),
        crlf: body.contains("\r\n"),
        bom,
        final_newline: body.ends_with('\n') || body.trim().is_empty(),
    };
    if body.trim().is_empty() {
        return Ok((Json::Object(Vec::new()), layout));
    }
    let value: Json = serde_json::from_str(body).context("it is not plain JSON")?;
    if !matches!(value, Json::Object(_)) {
        bail!("it is not a JSON object");
    }
    Ok((value, layout))
}

/// `value` written with `layout`.
pub(super) fn render(value: &Json, layout: &Layout) -> String {
    let mut bytes = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(layout.indent.as_bytes());
    let mut serializer = serde_json::Serializer::with_formatter(&mut bytes, formatter);
    value
        .serialize(&mut serializer)
        .expect("writing JSON to memory cannot fail");
    let mut text = String::from_utf8(bytes).expect("serde_json writes UTF-8");
    if layout.final_newline {
        text.push('\n');
    }
    // A string's own line breaks are escaped, so every newline is layout.
    if layout.crlf {
        text = text.replace('\n', "\r\n");
    }
    if layout.bom {
        text.insert(0, '\u{feff}');
    }
    text
}

/// The indentation of the first indented line: one level, since a pretty
/// document's second line is its first key. Two spaces when nothing is
/// indented.
fn indent(text: &str) -> String {
    text.lines()
        .skip(1)
        .map(|line| {
            let rest = line.trim_start_matches([' ', '\t']);
            &line[..line.len() - rest.len()]
        })
        .find(|whitespace| !whitespace.is_empty())
        .map_or_else(|| "  ".to_string(), str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(text: &str) -> String {
        let (value, layout) = parse(text).unwrap();
        render(&value, &layout)
    }

    #[test]
    fn keys_keep_their_order_at_every_level() {
        let text = "{\n  \"zeta\": 1,\n  \"alpha\": {\n    \"y\": [true, null, \"é\"],\n    \"b\": 2.5\n  },\n  \"middle\": {}\n}\n";
        let expanded = "{\n  \"zeta\": 1,\n  \"alpha\": {\n    \"y\": [\n      true,\n      null,\n      \"é\"\n    ],\n    \"b\": 2.5\n  },\n  \"middle\": {}\n}\n";
        assert_eq!(round_trip(text), expanded);
        assert_eq!(
            round_trip(expanded),
            expanded,
            "a pretty document is kept byte for byte"
        );
    }

    #[test]
    fn indentation_line_ends_bom_and_final_newline_are_kept() {
        for text in [
            "{\n    \"a\": {\n        \"b\": 1\n    }\n}\n",
            "{\n\t\"a\": {\n\t\t\"b\": 1\n\t}\n}\n",
            "{\r\n  \"a\": {\r\n    \"b\": 1\r\n  }\r\n}\r\n",
            "\u{feff}{\n  \"a\": 1\n}\n",
            "{\n  \"a\": 1\n}",
        ] {
            assert_eq!(round_trip(text), text, "{text:?}");
        }
    }

    #[test]
    fn empty_text_is_an_empty_object_and_other_json_is_refused() {
        let (value, layout) = parse(" \n").unwrap();
        assert!(value.is_empty());
        assert_eq!(render(&value, &layout), "{}\n");
        for text in ["[1]", "{\"a\": 1} // comment", "{\"a\":", "\"text\""] {
            assert!(parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn numbers_and_strings_keep_their_text() {
        let text = "{\n  \"days\": 1e3,\n  \"ratio\": 1.50,\n  \"big\": 123456789012345678901234567890,\n  \"name\": \"caf\\u00e9 \\/ path\",\n  \"list\": [\n    -0.0,\n    \"\\t\"\n  ]\n}\n";
        assert_eq!(round_trip(text), text);
        let (value, _) = parse(text).unwrap();
        assert_eq!(
            value.get("name").and_then(Json::as_str),
            Some("café / path")
        );
    }

    #[test]
    fn set_replaces_in_place_and_appends_new_keys() {
        let (mut value, layout) = parse("{\"a\": 1, \"b\": 2}").unwrap();
        value.set("a", "one".into());
        value.set("c", 3u64.into());
        assert_eq!(value.remove("b"), Some(2u64.into()));
        assert_eq!(
            render(&value, &layout),
            "{\n  \"a\": \"one\",\n  \"c\": 3\n}"
        );
    }
}
