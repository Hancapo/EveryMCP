pub fn mask(bits: u32) -> u128 {
    (1u128 << bits) - 1
}

pub fn signed(value: u128, bits: u32) -> i128 {
    if value >= 1u128 << (bits - 1) {
        value as i128 - (1i128 << bits)
    } else {
        value as i128
    }
}

pub fn reflect_bits(mut value: u128, bits: u32) -> u128 {
    let mut reflected = 0;
    for _ in 0..bits {
        reflected = (reflected << 1) | (value & 1);
        value >>= 1;
    }
    reflected
}

pub fn unsigned(value: i128, bits: u32) -> Result<u128, String> {
    if value < -(1i128 << (bits - 1)) || value > mask(bits) as i128 {
        return Err(format!("Value does not fit in {bits} bits."));
    }
    Ok((value as u128) & mask(bits))
}

pub fn hex(value: u128, min_digits: usize) -> String {
    format!("0x{value:0min_digits$X}")
}
pub fn binary(value: u128, bits: usize) -> String {
    format!("0b{value:0bits$b}")
}

pub fn bits(args: &serde_json::Value) -> Result<u32, String> {
    let value = crate::input::int(args, "bits")?;
    match value {
        8 | 16 | 32 | 64 => Ok(value as u32),
        _ => Err("'bits' must be 8, 16, 32 or 64.".into()),
    }
}

pub fn address(args: &serde_json::Value, name: &str) -> Result<u128, String> {
    unsigned(crate::input::integer(args, name)?, 64)
}
