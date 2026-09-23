use crate::input;
use half::f16;
use num_traits::ToPrimitive;
use serde_json::{Value, json};

pub fn parse_hex(raw: &str) -> Result<Vec<u8>, String> {
    let raw = raw.trim();
    let raw = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    let digits: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if !digits.len().is_multiple_of(2) || !digits.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Hex data must contain complete byte pairs.".into());
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&digits[i..i + 2], 16).map_err(|_| "Invalid hex byte.".to_owned())
        })
        .collect()
}

pub fn format_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn endian(name: &str) -> Result<bool, String> {
    match name {
        "little" => Ok(true),
        "big" => Ok(false),
        _ => Err("Endian must be 'little' or 'big'.".into()),
    }
}

#[derive(Clone, Copy)]
pub enum ScalarType {
    Unsigned(usize),
    Signed(usize),
    Float(usize),
}

impl ScalarType {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "u8" => Ok(Self::Unsigned(1)),
            "i8" => Ok(Self::Signed(1)),
            "u16" => Ok(Self::Unsigned(2)),
            "i16" => Ok(Self::Signed(2)),
            "u32" => Ok(Self::Unsigned(4)),
            "i32" => Ok(Self::Signed(4)),
            "u64" => Ok(Self::Unsigned(8)),
            "i64" => Ok(Self::Signed(8)),
            "f16" => Ok(Self::Float(2)),
            "f32" => Ok(Self::Float(4)),
            "f64" => Ok(Self::Float(8)),
            _ => Err("Unsupported scalar type.".into()),
        }
    }
    pub fn width(self) -> usize {
        match self {
            Self::Unsigned(n) | Self::Signed(n) | Self::Float(n) => n,
        }
    }
}

pub fn read_scalar(
    data: &[u8],
    offset: usize,
    kind: ScalarType,
    little: bool,
) -> Result<Value, String> {
    let width = kind.width();
    let end = offset.checked_add(width).ok_or("Byte offset overflows.")?;
    let bytes = data.get(offset..end).ok_or("Read exceeds input bytes.")?;
    let raw = if little {
        bytes
            .iter()
            .rev()
            .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte))
    } else {
        bytes
            .iter()
            .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte))
    };
    Ok(match kind {
        ScalarType::Unsigned(_) => json!(raw.to_string()),
        ScalarType::Signed(_) => {
            let bits = width * 8;
            let signed = if raw & (1u64 << (bits - 1)) != 0 {
                raw as i128 - (1i128 << bits)
            } else {
                raw as i128
            };
            json!(signed.to_string())
        }
        ScalarType::Float(2) => float_value(f16::from_bits(raw as u16).to_f32() as f64),
        ScalarType::Float(4) => float_value(f32::from_bits(raw as u32) as f64),
        ScalarType::Float(8) => float_value(f64::from_bits(raw)),
        _ => unreachable!(),
    })
}

fn float_value(number: f64) -> Value {
    if number.is_nan() {
        json!("NaN")
    } else if number == f64::INFINITY {
        json!("Infinity")
    } else if number == f64::NEG_INFINITY {
        json!("-Infinity")
    } else {
        json!(number)
    }
}

pub fn write_scalar(value: &Value, kind: ScalarType, little: bool) -> Result<Vec<u8>, String> {
    let width = kind.width();
    let raw = match kind {
        ScalarType::Unsigned(_) | ScalarType::Signed(_) => {
            let text = value.as_str().ok_or("Integer values must be strings.")?;
            let integer = input::parse_big_integer(text, "value")?;
            let bits = width * 8;
            let low = if matches!(kind, ScalarType::Signed(_)) {
                -(num_bigint::BigInt::from(1u8) << (bits - 1))
            } else {
                num_bigint::BigInt::from(0u8)
            };
            let high = if matches!(kind, ScalarType::Signed(_)) {
                (num_bigint::BigInt::from(1u8) << (bits - 1)) - 1u8
            } else {
                (num_bigint::BigInt::from(1u8) << bits) - 1u8
            };
            if integer < low || integer > high {
                return Err("Integer does not fit scalar type.".into());
            }
            let modulus = num_bigint::BigInt::from(1u8) << bits;
            let unsigned = if integer.sign() == num_bigint::Sign::Minus {
                integer + modulus
            } else {
                integer
            };
            unsigned
                .to_u64()
                .ok_or("Integer does not fit scalar type.")?
        }
        ScalarType::Float(_) => {
            let number = value
                .as_f64()
                .or_else(|| value.as_str().and_then(|s| s.parse::<f64>().ok()))
                .filter(|n| n.is_finite())
                .ok_or("Float must be finite.")?;
            match width {
                2 => {
                    let narrowed = f16::from_f64(number);
                    if !narrowed.is_finite() {
                        return Err("Float does not fit scalar type.".into());
                    }
                    narrowed.to_bits() as u64
                }
                4 => {
                    let narrowed = number as f32;
                    if !narrowed.is_finite() {
                        return Err("Float does not fit scalar type.".into());
                    }
                    narrowed.to_bits() as u64
                }
                8 => number.to_bits(),
                _ => unreachable!(),
            }
        }
    };
    let mut bytes = (0..width)
        .map(|i| (raw >> (i * 8)) as u8)
        .collect::<Vec<_>>();
    if !little {
        bytes.reverse();
    }
    Ok(bytes)
}
