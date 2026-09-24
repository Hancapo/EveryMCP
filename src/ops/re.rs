use crate::{input, integer};
use integer::{address, binary, bits, hex, mask, unsigned};
use serde_json::{Value, json};

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "integer_convert" => integer_convert(args),
        "bitfield_extract" => bitfield_extract(args),
        "bitfield_insert" => bitfield_insert(args),
        "address_translate" => address_translate(args),
        "align_address" => align_address(args),
        "relative_target" => relative_target(args),
        "ieee754_decode" => ieee754_decode(args),
        _ => Err(format!("Unknown tool '{name}'.")),
    }
}

fn integer_convert(args: &Value) -> Result<Value, String> {
    let bits = bits(args)?;
    let value = unsigned(input::integer(args, "value")?, bits)?;
    let signed = if value >= 1u128 << (bits - 1) {
        value as i128 - (1i128 << bits)
    } else {
        value as i128
    };
    let bytes: Vec<String> = (0..bits / 8)
        .map(|index| format!("{:02X}", (value >> (index * 8)) & 0xff))
        .collect();
    Ok(json!({
        "signed": signed.to_string(),
        "unsigned": value.to_string(),
        "hex": hex(value, (bits/4) as usize),
        "binary": binary(value, bits as usize),
        "littleEndianBytes": bytes.join(" "),
        "bigEndianBytes": bytes.iter().rev().map(String::as_str).collect::<Vec<_>>().join(" ")
    }))
}

fn field(args: &Value, bits: u32) -> Result<(u32, u32), String> {
    let offset = input::int(args, "offset")?;
    let width = input::int(args, "width")?;
    if offset < 0 || width <= 0 || offset >= bits as i32 || width > bits as i32 - offset {
        return Err("Bitfield must fit inside the word.".into());
    }
    Ok((offset as u32, width as u32))
}

fn bitfield_extract(args: &Value) -> Result<Value, String> {
    let value = unsigned(input::integer(args, "value")?, 64)?;
    let (offset, width) = field(args, 64)?;
    let extracted = (value >> offset) & mask(width);
    let signed = if extracted >= 1u128 << (width - 1) {
        extracted as i128 - (1i128 << width)
    } else {
        extracted as i128
    };
    Ok(
        json!({"unsigned":extracted.to_string(),"signed":signed.to_string(),"hex":hex(extracted, 1)}),
    )
}

fn bitfield_insert(args: &Value) -> Result<Value, String> {
    let bits = bits(args)?;
    let value = unsigned(input::integer(args, "value")?, bits)?;
    let (offset, width) = field(args, bits)?;
    let inserted = unsigned(input::integer(args, "field")?, width)?;
    let shifted = mask(width) << offset;
    let result = (value & (mask(bits) ^ shifted)) | (inserted << offset);
    Ok(json!({"unsigned":result.to_string(),"hex":hex(result, 1)}))
}

fn address_translate(args: &Value) -> Result<Value, String> {
    let virtual_address = address(args, "address")?;
    let image_base = address(args, "imageBase")?;
    let runtime_base = address(args, "runtimeBase")?;
    if virtual_address < image_base {
        return Err("Address is below imageBase.".into());
    }
    let rva = virtual_address - image_base;
    let rebased = runtime_base + rva;
    if rebased > mask(64) {
        return Err("Runtime address overflows 64 bits.".into());
    }
    Ok(json!({"rva":hex(rva, 1),"runtimeAddress":hex(rebased, 1)}))
}

fn align_address(args: &Value) -> Result<Value, String> {
    let virtual_address = address(args, "address")?;
    let alignment = address(args, "alignment")?;
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err("Alignment must be a nonzero power of two.".into());
    }
    let down = integer::align_down(virtual_address, alignment).unwrap();
    let up = integer::align_up(virtual_address, alignment)
        .ok_or("Aligned address overflows 128 bits.")?;
    if up > mask(64) {
        return Err("Aligned address overflows 64 bits.".into());
    }
    Ok(json!({"down":hex(down, 1),"up":hex(up, 1),"padding":(up-virtual_address).to_string()}))
}

fn relative_target(args: &Value) -> Result<Value, String> {
    let instruction_address = address(args, "instructionAddress")?;
    let size = input::int(args, "instructionSize")?;
    if size <= 0 {
        return Err("Instruction size must be positive.".into());
    }
    let displacement = input::integer(args, "displacement")?;
    let target = (instruction_address as i128 + size as i128)
        .checked_add(displacement)
        .ok_or_else(|| "Relative target overflows 64 bits.".to_owned())?;
    if target < 0 || target as u128 > mask(64) {
        return Err("Relative target overflows 64 bits.".into());
    }
    Ok(json!({"target":hex(target as u128, 1)}))
}

fn ieee754_decode(args: &Value) -> Result<Value, String> {
    let bits = input::int(args, "bits")?;
    if bits != 32 && bits != 64 {
        return Err("'bits' must be 32 or 64.".into());
    }
    let pattern = unsigned(input::integer(args, "pattern")?, bits as u32)? as u64;
    let number = if bits == 32 {
        f32::from_bits(pattern as u32) as f64
    } else {
        f64::from_bits(pattern)
    };
    let exponent_bits = if bits == 32 { 8 } else { 11 };
    let fraction_bits = if bits == 32 { 23 } else { 52 };
    let exponent = (pattern >> fraction_bits) & ((1u64 << exponent_bits) - 1);
    let fraction = pattern & ((1u64 << fraction_bits) - 1);
    let classification = if exponent == (1u64 << exponent_bits) - 1 {
        if fraction == 0 { "infinity" } else { "nan" }
    } else if exponent == 0 {
        if fraction == 0 { "zero" } else { "subnormal" }
    } else {
        "normal"
    };
    let display = if number.is_nan() {
        "NaN".to_owned()
    } else if number == f64::INFINITY {
        "Infinity".to_owned()
    } else if number == f64::NEG_INFINITY {
        "-Infinity".to_owned()
    } else {
        number.to_string()
    };
    Ok(
        json!({"value":display,"classification":classification,"sign":pattern >> (bits-1),"exponent":exponent,"fractionHex":hex(fraction as u128, 1)}),
    )
}
