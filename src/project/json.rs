use std::io::{self, Read};

/// Bound individual decoded strings before serde_json allocates their complete contents.
/// Source/repair arrays and the complete document remain streaming and uncapped.
pub(super) fn bounded_strings(input: impl Read) -> impl Read {
    StringBudget::new(input, crate::looks::MAX_EMBEDDED_TEXT_BYTES)
}

#[derive(Clone, Copy)]
enum Escape {
    None,
    Start,
    Unicode { remaining: u8, value: u16 },
}

struct StringBudget<R> {
    input: R,
    limit: usize,
    quoted: bool,
    escape: Escape,
    bytes: usize,
    failed: bool,
}

impl<R: Read> StringBudget<R> {
    fn new(input: R, limit: usize) -> Self {
        Self {
            input,
            limit,
            quoted: false,
            escape: Escape::None,
            bytes: 0,
            failed: false,
        }
    }

    fn inspect(&mut self, byte: u8) -> io::Result<()> {
        if !self.quoted {
            if byte == b'"' {
                self.quoted = true;
                self.bytes = 0;
                self.escape = Escape::None;
            }
            return Ok(());
        }

        let added = match self.escape {
            Escape::Start if byte == b'u' => {
                self.escape = Escape::Unicode {
                    remaining: 4,
                    value: 0,
                };
                0
            }
            Escape::Start => {
                self.escape = Escape::None;
                1
            }
            Escape::Unicode { remaining, value } => {
                let digit = (byte as char).to_digit(16).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid JSON Unicode escape")
                })?;
                let value = (value << 4) | digit as u16;
                if remaining > 1 {
                    self.escape = Escape::Unicode {
                        remaining: remaining - 1,
                        value,
                    };
                    0
                } else {
                    self.escape = Escape::None;
                    match value {
                        0..=0x7f => 1,
                        0x80..=0x7ff => 2,
                        // Valid pairs decode to four UTF-8 bytes, charged at the high surrogate.
                        // serde_json remains responsible for rejecting unmatched surrogates.
                        0xd800..=0xdbff => 4,
                        0xdc00..=0xdfff => 0,
                        _ => 3,
                    }
                }
            }
            Escape::None => match byte {
                b'"' => {
                    self.quoted = false;
                    0
                }
                b'\\' => {
                    self.escape = Escape::Start;
                    0
                }
                _ => 1,
            },
        };

        self.bytes = self
            .bytes
            .checked_add(added)
            .filter(|bytes| *bytes <= self.limit)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "project JSON string exceeds the supported byte budget",
                )
            })?;
        Ok(())
    }
}

impl<R: Read> Read for StringBudget<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.failed {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "project JSON string budget rejected input",
            ));
        }
        let count = self.input.read(buffer)?;
        for &byte in &buffer[..count] {
            if let Err(error) = self.inspect(byte) {
                self.failed = true;
                return Err(error);
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        io::{BufReader, Cursor},
        rc::Rc,
    };

    #[test]
    fn oversized_stream_rejects_before_materializing_the_whole_string() {
        struct Count<R> {
            input: R,
            bytes: Rc<Cell<usize>>,
        }

        impl<R: Read> Read for Count<R> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let count = self.input.read(buffer)?;
                self.bytes.set(self.bytes.get() + count);
                Ok(count)
            }
        }

        let bytes = Rc::new(Cell::new(0));
        let input = Cursor::new(b"{\"samples\":\"")
            .chain(io::repeat(b'a').take(10_000_000))
            .chain(Cursor::new(b"\"}"));
        let counted = Count {
            input,
            bytes: bytes.clone(),
        };
        let result = serde_json::from_reader::<_, serde_json::Value>(BufReader::with_capacity(
            256,
            StringBudget::new(counted, 32),
        ));
        assert!(result.unwrap_err().to_string().contains("byte budget"));
        assert!(
            bytes.get() <= 512,
            "oversized input was consumed before admission"
        );
    }

    #[test]
    fn string_budget_counts_decoded_utf8_and_resets_between_tokens() {
        for (input, expected) in [
            (r#""\u0041\u0042\u0043\u0044""#, "ABCD"),
            (r#""\uD83D\uDE00""#, "😀"),
            (r#""\"\\\/\n""#, "\"\\/\n"),
            (r#""😀""#, "😀"),
        ] {
            let value: String =
                serde_json::from_reader(StringBudget::new(input.as_bytes(), 4)).unwrap();
            assert_eq!(value, expected);
            assert!(
                serde_json::from_reader::<_, String>(StringBudget::new(input.as_bytes(), 3))
                    .is_err()
            );
        }
        let input = serde_json::to_vec(&vec!["abcd"; 128]).unwrap();
        let values: Vec<String> =
            serde_json::from_reader(StringBudget::new(input.as_slice(), 4)).unwrap();
        assert_eq!(values.len(), 128);
    }

    #[test]
    fn malformed_unicode_remains_invalid_json() {
        for input in [
            r#""\uDE00""#,
            r#""\uD83Dx""#,
            r#""\uGGGG""#,
            r#""\uD83D\u0041""#,
        ] {
            assert!(
                serde_json::from_reader::<_, String>(StringBudget::new(input.as_bytes(), 8))
                    .is_err()
            );
        }
    }
}
