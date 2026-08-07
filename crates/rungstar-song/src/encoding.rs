//! Working out what bytes on disk actually say.
//!
//! Song files predate any convention. Modern ones are UTF-8, often with a byte-order mark;
//! older ones are whatever code page the author's machine used, most commonly Windows-1252.
//! UltraStar Deluxe resolves this with a sniff-then-fall-back rule, which is reproduced here.

use std::fmt;

/// Text encodings a song file may be written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Encoding {
    /// Sniff: valid UTF-8 is UTF-8, anything else is Windows-1252.
    #[default]
    Auto,
    Utf8,
    /// UTF-8 with a leading byte-order mark. Only distinguished so writing can restore it.
    Utf8Bom,
    /// Windows-1252, the western European code page.
    Cp1252,
    /// Windows-1250, the central European code page.
    Cp1250,
}

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

impl Encoding {
    /// Interpret an `#ENCODING` value. Unknown names fall back to [`Encoding::Auto`].
    pub fn parse(value: &str) -> Self {
        match value
            .trim()
            .to_ascii_uppercase()
            .replace(['-', '_'], "")
            .as_str()
        {
            "UTF8" => Self::Utf8,
            "CP1252" | "WINDOWS1252" | "ANSI" => Self::Cp1252,
            "CP1250" | "WINDOWS1250" => Self::Cp1250,
            _ => Self::Auto,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::Utf8 | Self::Utf8Bom => "UTF8",
            Self::Cp1252 => "CP1252",
            Self::Cp1250 => "CP1250",
        }
    }
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The outcome of decoding a file's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    /// What the bytes turned out to be, never [`Encoding::Auto`].
    pub encoding: Encoding,
}

/// Decode song bytes, honouring a BOM and otherwise sniffing.
///
/// `declared` is the `#ENCODING` header if one was found on a first pass. It is only
/// consulted when there is no BOM, because a BOM is unambiguous and a mislabelled header is
/// not.
pub fn decode(bytes: &[u8], declared: Encoding) -> Decoded {
    if let Some(rest) = bytes.strip_prefix(&UTF8_BOM) {
        return Decoded {
            text: String::from_utf8_lossy(rest).into_owned(),
            encoding: Encoding::Utf8Bom,
        };
    }
    match declared {
        Encoding::Utf8 | Encoding::Utf8Bom => Decoded {
            text: String::from_utf8_lossy(bytes).into_owned(),
            encoding: Encoding::Utf8,
        },
        Encoding::Cp1252 => Decoded {
            text: decode_with(bytes, encoding_rs::WINDOWS_1252),
            encoding: Encoding::Cp1252,
        },
        Encoding::Cp1250 => Decoded {
            text: decode_with(bytes, encoding_rs::WINDOWS_1250),
            encoding: Encoding::Cp1250,
        },
        Encoding::Auto => sniff(bytes),
    }
}

/// Valid UTF-8 is taken at face value; anything else is assumed to be Windows-1252.
///
/// Windows-1252 maps every byte, so this can never fail — which is exactly why it is the
/// fallback rather than a stricter code page.
pub fn sniff(bytes: &[u8]) -> Decoded {
    match std::str::from_utf8(bytes) {
        Ok(text) => Decoded {
            text: text.to_owned(),
            encoding: Encoding::Utf8,
        },
        Err(_) => Decoded {
            text: decode_with(bytes, encoding_rs::WINDOWS_1252),
            encoding: Encoding::Cp1252,
        },
    }
}

fn decode_with(bytes: &[u8], enc: &'static encoding_rs::Encoding) -> String {
    let (text, _) = enc.decode_without_bom_handling(bytes);
    text.into_owned()
}

/// Encode text for writing, adding a BOM when the encoding calls for one.
pub fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8Bom => {
            let mut out = UTF8_BOM.to_vec();
            out.extend_from_slice(text.as_bytes());
            out
        }
        Encoding::Cp1252 => encoding_rs::WINDOWS_1252.encode(text).0.into_owned(),
        Encoding::Cp1250 => encoding_rs::WINDOWS_1250.encode(text).0.into_owned(),
        Encoding::Utf8 | Encoding::Auto => text.as_bytes().to_vec(),
    }
}

/// Split decoded text into lines, accepting LF, CRLF and lone CR terminators.
///
/// Trailing spaces are left alone: in a note line they are part of the lyric and dropping
/// them would run syllables together.
pub fn split_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                out.push(&text[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                out.push(&text[start..i]);
                i += if bytes.get(i + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        out.push(&text[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bom_forces_utf8_and_is_stripped() {
        let mut bytes = UTF8_BOM.to_vec();
        bytes.extend_from_slice("#TITLE:x".as_bytes());
        let decoded = decode(&bytes, Encoding::Cp1252);
        assert_eq!(decoded.text, "#TITLE:x");
        assert_eq!(decoded.encoding, Encoding::Utf8Bom);
    }

    #[test]
    fn valid_utf8_is_detected() {
        let decoded = sniff("Grüße – ünïcode".as_bytes());
        assert_eq!(decoded.encoding, Encoding::Utf8);
        assert_eq!(decoded.text, "Grüße – ünïcode");
    }

    #[test]
    fn invalid_utf8_falls_back_to_cp1252() {
        // 0xFC is "ü" in CP1252 but is not valid UTF-8 on its own.
        let decoded = sniff(b"Gr\xFC\xDFe");
        assert_eq!(decoded.encoding, Encoding::Cp1252);
        assert_eq!(decoded.text, "Grüße");
    }

    #[test]
    fn line_splitting_handles_every_terminator() {
        assert_eq!(split_lines("a\nb\r\nc\rd"), vec!["a", "b", "c", "d"]);
        assert_eq!(split_lines("a\n"), vec!["a"]);
        assert_eq!(split_lines(""), Vec::<&str>::new());
        // A blank first line must survive as an empty entry so it can be rejected.
        assert_eq!(split_lines("\na"), vec!["", "a"]);
    }

    #[test]
    fn trailing_spaces_in_lyrics_survive() {
        assert_eq!(
            split_lines(": 0 1 0 word \n- 4"),
            vec![": 0 1 0 word ", "- 4"]
        );
    }
}
