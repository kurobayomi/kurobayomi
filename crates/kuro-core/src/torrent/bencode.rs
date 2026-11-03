//! Strict bencode parser and writer (port of `BencodeParser.kt` and `BencodeWriter.kt`).
//!
//! Deviation D-7: dictionary keys compare as unsigned bytes (BEP 3), so `d1:ai1e1:\x80i2ee` parses; the Kotlin
//! compares signed bytes and rejects it.

use super::TorrentError;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    /// Keys in unsigned byte order, which is the order `BTreeMap<Vec<u8>, _>` iterates in.
    Dict(BTreeMap<Vec<u8>, Value>),
}

impl Value {
    /// `Dictionary.getByString`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Dict(d) => d.get(key.as_bytes()),
            _ => None,
        }
    }
}

fn err(msg: &str) -> TorrentError {
    TorrentError::Bencode(msg.to_string())
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn next(&mut self) -> Result<u8, TorrentError> {
        let b = *self.input.get(self.pos).ok_or_else(|| err("Unexpected end of input"))?;
        self.pos += 1;
        Ok(b)
    }

    fn value(&mut self, head: u8) -> Result<Value, TorrentError> {
        match head {
            b'i' => {
                let first = self.next()?;
                Ok(Value::Int(self.number(first, b'e')?))
            }
            b'0'..=b'9' => Ok(Value::Bytes(self.byte_string(head)?)),
            b'l' => {
                let mut items = Vec::new();
                loop {
                    let b = self.next()?;
                    if b == b'e' {
                        break;
                    }
                    items.push(self.value(b)?);
                }
                Ok(Value::List(items))
            }
            b'd' => {
                let mut map = BTreeMap::new();
                loop {
                    let b = self.next()?;
                    if b == b'e' {
                        break;
                    }
                    let key = self.byte_string(b)?;
                    // strictly ascending: out-of-order and duplicate keys are both rejected
                    if map.last_key_value().is_some_and(|(last, _)| *last >= key) {
                        return Err(err("Dictionary keys out of order"));
                    }
                    let head = self.next()?;
                    map.insert(key, self.value(head)?);
                }
                Ok(Value::Dict(map))
            }
            _ => Err(err("Unexpected value type")),
        }
    }

    fn byte_string(&mut self, head: u8) -> Result<Vec<u8>, TorrentError> {
        let len = self.number(head, b':')?;
        if !(0..=i64::from(i32::MAX)).contains(&len) {
            return Err(err("Invalid string length"));
        }
        let len = len as usize;
        let end = self.pos.checked_add(len).filter(|e| *e <= self.input.len()).ok_or_else(|| err("Unexpected end of input"))?;
        let out = self.input[self.pos..end].to_vec();
        self.pos = end;
        Ok(out)
    }

    /// Digits with an optional leading `-`; no leading zeros, no `-0`; ends at `terminator`.
    fn number(&mut self, head: u8, terminator: u8) -> Result<i64, TorrentError> {
        let mut text = String::new();
        let mut b = head;
        loop {
            let ok = match text.as_str() {
                "" => b == b'-' || b.is_ascii_digit(),
                "0" => false,
                "-" => (b'1'..=b'9').contains(&b),
                _ => b.is_ascii_digit(),
            };
            if !ok {
                return Err(err("Unexpected integer character"));
            }
            text.push(b as char);
            b = self.next()?;
            if b == terminator {
                break;
            }
        }
        text.parse::<i64>().map_err(|_| err("Integer out of range"))
    }
}

/// Parses one value and requires the whole input to be consumed.
pub fn parse(input: &[u8]) -> Result<Value, TorrentError> {
    let mut p = Parser { input, pos: 0 };
    let head = p.next()?;
    let v = p.value(head)?;
    if p.pos != input.len() {
        return Err(err("Unexpected extra data"));
    }
    Ok(v)
}

/// Writes a value. A `BTreeMap` is always in order, so this cannot fail; the `Result` keeps the signature uniform.
pub fn encode(v: &Value) -> Result<Vec<u8>, TorrentError> {
    let mut out = Vec::new();
    write(v, &mut out);
    Ok(out)
}

fn write(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::Int(n) => {
            out.push(b'i');
            out.extend_from_slice(n.to_string().as_bytes());
            out.push(b'e');
        }
        Value::Bytes(b) => write_bytes(b, out),
        Value::List(items) => {
            out.push(b'l');
            items.iter().for_each(|i| write(i, out));
            out.push(b'e');
        }
        Value::Dict(map) => {
            out.push(b'd');
            for (k, v) in map {
                write_bytes(k, out);
                write(v, out);
            }
            out.push(b'e');
        }
    }
}

fn write_bytes(b: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(b.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(b);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(pairs: &[(&[u8], Value)]) -> Value {
        Value::Dict(pairs.iter().map(|(k, v)| (k.to_vec(), v.clone())).collect())
    }

    fn bytes(b: &[u8]) -> Value {
        Value::Bytes(b.to_vec())
    }

    #[test]
    fn accepted_rows_of_the_section_11_1_table() {
        assert_eq!(parse(b"i0e").unwrap(), Value::Int(0));
        assert_eq!(parse(b"i-5e").unwrap(), Value::Int(-5));
        assert_eq!(parse(b"0:").unwrap(), bytes(b""));
        assert_eq!(parse(b"3:abc").unwrap(), bytes(b"abc"));
        assert_eq!(parse(b"li1ee").unwrap(), Value::List(vec![Value::Int(1)]));
        assert_eq!(parse(b"le").unwrap(), Value::List(vec![]));
        assert_eq!(parse(b"d1:ai1e1:bi2ee").unwrap(), dict(&[(b"a", Value::Int(1)), (b"b", Value::Int(2))]));
    }

    #[test]
    fn rejected_rows_of_the_section_11_1_table() {
        for bad in [&b"i-0e"[..], b"i03e", b"ie", b"i1", b"3:ab", b"d1:bi2e1:ai1ee", b"d1:ai1e1:ai2ee", b"i1eX", b"x", b""] {
            assert!(parse(bad).is_err(), "{:?} must be rejected", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn unsorted_keys_give_the_out_of_order_message() {
        assert_eq!(parse(b"d1:bi2e1:ai1ee"), Err(TorrentError::Bencode("Dictionary keys out of order".into())));
        assert_eq!(parse(b"x"), Err(TorrentError::Bencode("Unexpected value type".into())));
        assert_eq!(parse(b"i1eX"), Err(TorrentError::Bencode("Unexpected extra data".into())));
    }

    #[test]
    fn keys_compare_as_unsigned_bytes() {
        let v = parse(b"d1:ai1e1:\x80i2ee").unwrap();
        assert_eq!(v, dict(&[(b"a", Value::Int(1)), (b"\x80", Value::Int(2))]));
    }

    #[test]
    fn integer_overflow_and_huge_string_lengths_are_errors() {
        assert!(parse(b"i9223372036854775808e").is_err());
        assert_eq!(parse(b"i9223372036854775807e").unwrap(), Value::Int(i64::MAX));
        assert!(parse(b"2147483648:").is_err());
        assert!(parse(b"2147483647:").is_err(), "in range but the input ends first");
    }

    #[test]
    fn encode_is_the_inverse_of_parse() {
        for src in [&b"d1:ai1e1:\x80i2ee"[..], b"l3:abci-7ed1:xleee", b"0:", b"le"] {
            assert_eq!(encode(&parse(src).unwrap()).unwrap(), src);
        }
    }
}
