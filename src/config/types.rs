use std::borrow::Cow;
use std::fmt;

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{de, Deserialize};

#[derive(Debug, Copy, Clone)]
pub enum BinaryUnit {
    /// A single byte.
    Byte,

    /// 2^10 (1024) bytes
    Kibi,

    /// 2^20 (1024*1024) bytes
    Mebi,

    /// 2^30 (1024*1024*1024) bytes
    Gibi,
}

impl BinaryUnit {
    /// The unit in podman's `b`/`k`/`m`/`g` notation.
    pub fn as_podman(&self) -> char {
        match self {
            BinaryUnit::Byte => 'b',
            BinaryUnit::Kibi => 'k',
            BinaryUnit::Mebi => 'm',
            BinaryUnit::Gibi => 'g',
        }
    }

    pub fn bytes(self) -> usize {
        match self {
            BinaryUnit::Byte => 1,
            BinaryUnit::Kibi => 1024,
            BinaryUnit::Mebi => 1024 * 1024,
            BinaryUnit::Gibi => 1024 * 1024 * 1024,
        }
    }
}

/// Specifier for a readable value, e.g. `256m` or `2G`. Spaces and cases are
/// ignored when deriving it from a string, such that `256m` and `   256  m  `
/// parse as the exact same thing.
#[derive(Debug, Copy, Clone)]
pub struct ByteSize {
    value: u16,
    unit: BinaryUnit,
}

impl ByteSize {
    pub fn bytes(&self) -> usize {
        (self.value as usize) * self.unit.bytes()
    }

    /// The size in podman's `b`/`k`/`m`/`g` notation.
    pub fn fmt_podman(&self, f: &mut impl std::fmt::Write) -> fmt::Result {
        write!(f, "{}{}", self.value, self.unit.as_podman())
    }

    pub fn to_podman_string(&self) -> String {
        format!("{}{}", self.value, self.unit.as_podman())
    }
}

impl fmt::Display for ByteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_podman(f)
    }
}

impl std::str::FromStr for ByteSize {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        // first strip all whitespaces
        let s: String = s.split_whitespace().collect();
        let Some((i, suffix)) = s.char_indices().next_back() else {
            return Err("empty size".to_string());
        };
        let unit = match suffix.to_ascii_lowercase() {
            'b' => BinaryUnit::Byte,
            'k' => BinaryUnit::Kibi,
            'm' => BinaryUnit::Mebi,
            'g' => BinaryUnit::Gibi,
            _ => return Err(format!("`{s}` has no `b`/`k`/`m`/`g` suffix")),
        };
        let digits = &s[..i];
        match digits.parse() {
            Ok(value) => Ok(ByteSize { value, unit }),
            Err(_) => Err(format!("`{digits}` is not a number below 65536")),
        }
    }
}

impl<'de> Deserialize<'de> for ByteSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = Cow::<str>::deserialize(deserializer)?;
        s.parse().map_err(de::Error::custom)
    }
}

impl JsonSchema for ByteSize {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ByteSize")
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": r"^\s*[0-9]{1,5}\s*[bkmgBKMG]\s*$",
            "examples": ["256m", "2G", " 64 k "],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bytesize_parsing() {
        fn parsed(s: &str) -> ByteSize {
            s.parse().expect("valid size")
        }

        assert_eq!(parsed("1b").bytes(), 1);
        assert_eq!(parsed("8K").bytes(), 8 * 1024);
        assert_eq!(parsed("256m").bytes(), 256 * 1024 * 1024);
        assert_eq!(parsed(" \n 256\n\t m   ").bytes(), 256 * 1024 * 1024);
        assert_eq!(parsed("2G").bytes(), 2 * 1024 * 1024 * 1024);
        assert_eq!(parsed("2G").to_podman_string(), "2g");

        for s in ["", "1024", "2t", "m", "abcm", "-2m", "65536m"] {
            assert!(s.parse::<ByteSize>().is_err(), "`{s}` should not parse");
        }
    }

    #[test]
    fn test_bytesize_deserialize() {
        let size = serde_json::from_str::<ByteSize>("\"2G\"").expect("valid size");
        assert_eq!(size.bytes(), 2 * 1024 * 1024 * 1024);
        assert!(serde_json::from_str::<ByteSize>("\"nonsense\"").is_err());
        assert!(serde_json::from_str::<ByteSize>("2048").is_err());
    }
}
