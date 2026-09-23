use crate::{bytes, input, integer};
use integer::{address, bits, hex, mask, signed, unsigned};
use num_bigint::{BigInt, Sign};
use num_traits::{One, ToPrimitive, Zero};
use serde_json::{Value, json};

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "pe_address_map" => pe_address_map(args),
        "binary_unpack" => binary_unpack(args),
        "binary_pack" => binary_pack(args),
        "bitwise_word" => bitwise_word(args),
        "fixed_width_alu" => fixed_width_alu(args),
        "leb128_codec" => leb128_codec(args),
        "x86_effective_address" => x86_effective_address(args),
        "pe_relocation_apply" => pe_relocation_apply(args),
        "packed_vertex_decode" => packed_vertex_decode(args),
        "crc_compute" => crc_compute(args),
        _ => Err(format!("Unknown tool '{name}'.")),
    }
}

fn pe_address_map(args: &Value) -> Result<Value, String> {
    let mode = input::string(args, "mode")?;
    let requested = address(args, "value")?;
    let base = address(args, "imageBase")?;
    let headers = address(args, "sizeOfHeaders")?;
    let sections = input::property(args, "sections")?
        .as_array()
        .ok_or("'sections' must be an array.")?;
    if sections.len() > 128 {
        return Err("Too many sections.".into());
    }
    let file_to_rva = matches!(mode, "file_to_rva" | "file_to_va");
    let rva_input = match mode {
        "rva_to_file" => Some(requested),
        "va_to_file" => Some(
            requested
                .checked_sub(base)
                .ok_or("VA is below imageBase.")?,
        ),
        "file_to_rva" | "file_to_va" => None,
        _ => return Err("Unsupported PE address mapping mode.".into()),
    };
    let mut mapped = None;
    if let Some(rva) = rva_input {
        if rva < headers {
            mapped = Some((rva, rva, "headers".to_owned()));
        }
    } else if requested < headers {
        mapped = Some((requested, requested, "headers".to_owned()));
    }
    if mapped.is_none() {
        for (index, section) in sections.iter().enumerate() {
            let va = address(section, "virtualAddress")?;
            let virtual_size = address(section, "virtualSize")?;
            let raw = address(section, "pointerToRawData")?;
            let raw_size = address(section, "sizeOfRawData")?;
            let section_name = section
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("section_{index}"));
            if file_to_rva {
                if requested >= raw && requested - raw < raw_size {
                    mapped = Some((va + requested - raw, requested, section_name));
                    break;
                }
            } else {
                let rva = rva_input.unwrap();
                if rva >= va && rva - va < virtual_size.max(raw_size) {
                    let delta = rva - va;
                    if delta >= raw_size {
                        return Err("RVA is zero-filled and has no file offset.".into());
                    }
                    mapped = Some((rva, raw + delta, section_name));
                    break;
                }
            }
        }
    }
    let (rva, file, section) = mapped.ok_or("Address is not mapped by PE headers or sections.")?;
    if rva > mask(64) || file > mask(64) {
        return Err("Mapped RVA or file offset overflows 64 bits.".into());
    }
    let virtual_address = base
        .checked_add(rva)
        .filter(|n| *n <= mask(64))
        .ok_or("VA overflows 64 bits.")?;
    Ok(
        json!({"rva":hex(rva,1),"fileOffset":hex(file,1),"virtualAddress":hex(virtual_address,1),"section":section}),
    )
}

fn binary_unpack(args: &Value) -> Result<Value, String> {
    let data = bytes::parse_hex(input::string(args, "data")?)?;
    let kind = bytes::ScalarType::parse(input::string(args, "type")?)?;
    let little = bytes::endian(input::string(args, "endian")?)?;
    let offset = input::int(args, "offset")?;
    let count = input::int(args, "count")?;
    let stride = input::optional_int(args, "stride")?.unwrap_or(kind.width() as i32);
    if offset < 0 || !(1..=1024).contains(&count) || stride < kind.width() as i32 {
        return Err("Invalid offset, count or stride.".into());
    }
    let values = (0..count as usize)
        .map(|i| {
            let location = (offset as usize)
                .checked_add(
                    i.checked_mul(stride as usize)
                        .ok_or("Byte offset overflows.")?,
                )
                .ok_or("Byte offset overflows.")?;
            bytes::read_scalar(&data, location, kind, little)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"values":values}))
}

fn binary_pack(args: &Value) -> Result<Value, String> {
    let values = input::property(args, "values")?
        .as_array()
        .ok_or("'values' must be an array.")?;
    if values.is_empty() || values.len() > 1024 {
        return Err("'values' must contain 1 to 1024 elements.".into());
    }
    let kind = bytes::ScalarType::parse(input::string(args, "type")?)?;
    let little = bytes::endian(input::string(args, "endian")?)?;
    let mut output = Vec::with_capacity(values.len() * kind.width());
    for value in values {
        output.extend(bytes::write_scalar(value, kind, little)?);
    }
    Ok(json!({"data":bytes::format_hex(&output),"byteLength":output.len()}))
}

fn bitwise_word(args: &Value) -> Result<Value, String> {
    let operation = input::string(args, "operation")?;
    let width = bits(args)?;
    let a = unsigned(input::integer(args, "a")?, width)?;
    let result = match operation {
        "and" | "or" | "xor" => {
            let b = unsigned(input::integer(args, "b")?, width)?;
            match operation {
                "and" => a & b,
                "or" => a | b,
                _ => a ^ b,
            }
        }
        "not" => !a & mask(width),
        "shift_left" | "shift_right" | "rotate_left" | "rotate_right" => {
            let shift = input::int(args, "shift")?;
            if shift < 0 {
                return Err("Shift must be nonnegative.".into());
            }
            let shift = shift as u32;
            match operation {
                "shift_left" => {
                    if shift >= width {
                        0
                    } else {
                        (a << shift) & mask(width)
                    }
                }
                "shift_right" => {
                    if shift >= width {
                        0
                    } else {
                        a >> shift
                    }
                }
                "rotate_left" => {
                    let n = shift % width;
                    if n == 0 {
                        a
                    } else {
                        ((a << n) | (a >> (width - n))) & mask(width)
                    }
                }
                _ => {
                    let n = shift % width;
                    if n == 0 {
                        a
                    } else {
                        ((a >> n) | (a << (width - n))) & mask(width)
                    }
                }
            }
        }
        _ => return Err("Unsupported bitwise operation.".into()),
    };
    Ok(
        json!({"unsigned":result.to_string(),"hex":hex(result,(width/4) as usize),"binary":integer::binary(result,width as usize)}),
    )
}

fn fixed_width_alu(args: &Value) -> Result<Value, String> {
    let operation = input::string(args, "operation")?;
    let width = bits(args)?;
    let a = unsigned(input::integer(args, "a")?, width)?;
    let b = unsigned(input::integer(args, "b")?, width)?;
    let raw = match operation {
        "add" => a as i128 + b as i128,
        "subtract" => a as i128 - b as i128,
        "multiply" => (a * b) as i128,
        _ => return Err("Unsupported ALU operation.".into()),
    };
    let result = (raw as u128) & mask(width);
    let signed_result = signed(result, width);
    let signed_raw = match operation {
        "add" => signed(a, width) + signed(b, width),
        "subtract" => signed(a, width) - signed(b, width),
        _ => signed(a, width) * signed(b, width),
    };
    let signed_min = -(1i128 << (width - 1));
    let signed_max = (1i128 << (width - 1)) - 1;
    let carry = match operation {
        "add" => a + b > mask(width),
        "subtract" => a >= b,
        _ => a * b > mask(width),
    };
    Ok(
        json!({"unsigned":result.to_string(),"signed":signed_result.to_string(),"hex":hex(result,(width/4) as usize),
        "carry":carry,"overflow":signed_raw<signed_min || signed_raw>signed_max,
        "zero":result==0,"sign":result&(1u128<<(width-1))!=0}),
    )
}

fn leb128_codec(args: &Value) -> Result<Value, String> {
    let mode = input::string(args, "mode")?;
    let signed = input::boolean(args, "signed")?;
    match mode {
        "encode" => {
            let mut value = input::big_integer(args, "value")?;
            if !signed && value.sign() == Sign::Minus {
                return Err("ULEB128 value must be nonnegative.".into());
            }
            let mut output = Vec::new();
            loop {
                let mut byte = (&value & BigInt::from(0x7fu8)).to_u8().unwrap();
                value >>= 7;
                let done = if signed {
                    (value.is_zero() && byte & 0x40 == 0)
                        || (value == -BigInt::one() && byte & 0x40 != 0)
                } else {
                    value.is_zero()
                };
                if !done {
                    byte |= 0x80;
                }
                output.push(byte);
                if done {
                    break;
                }
                if output.len() >= 64 {
                    return Err("LEB128 value exceeds 64 bytes.".into());
                }
            }
            Ok(json!({"data":bytes::format_hex(&output),"byteLength":output.len()}))
        }
        "decode" => {
            let data = bytes::parse_hex(input::string(args, "data")?)?;
            let mut value = BigInt::zero();
            for (index, byte) in data.iter().copied().enumerate() {
                if index >= 64 {
                    return Err("LEB128 value exceeds 64 bytes.".into());
                }
                value += BigInt::from(byte & 0x7f) << (index * 7);
                if byte & 0x80 == 0 {
                    if signed && byte & 0x40 != 0 {
                        value -= BigInt::one() << ((index + 1) * 7);
                    }
                    return Ok(json!({"value":value.to_string(),"bytesConsumed":index+1}));
                }
            }
            Err("Incomplete LEB128 sequence.".into())
        }
        _ => Err("Mode must be encode or decode.".into()),
    }
}

fn x86_effective_address(args: &Value) -> Result<Value, String> {
    let width = input::int(args, "bits")?;
    if !matches!(width, 16 | 32 | 64) {
        return Err("Address bits must be 16, 32 or 64.".into());
    }
    let scale = input::int(args, "scale")?;
    if !matches!(scale, 1 | 2 | 4 | 8) {
        return Err("Scale must be 1, 2, 4 or 8.".into());
    }
    let base = input::integer(args, "base")?;
    let index = input::integer(args, "index")?;
    let displacement = input::integer(args, "displacement")?;
    let full = base
        .checked_add(
            index
                .checked_mul(scale as i128)
                .ok_or("Effective address overflows.")?,
        )
        .and_then(|n| n.checked_add(displacement))
        .ok_or("Effective address overflows.")?;
    let modulo = 1i128 << width;
    let result = full.rem_euclid(modulo) as u128;
    Ok(json!({"hex":hex(result,1),"unsigned":result.to_string()}))
}

fn pe_relocation_apply(args: &Value) -> Result<Value, String> {
    let kind = input::string(args, "type")?;
    let width = match kind {
        "HIGHLOW" => 32,
        "DIR64" => 64,
        _ => return Err("Type must be HIGHLOW or DIR64.".into()),
    };
    let value = unsigned(input::integer(args, "value")?, width)?;
    let old_base = address(args, "oldBase")? as i128;
    let new_base = address(args, "newBase")? as i128;
    let result = (value as i128 + new_base - old_base).rem_euclid(1i128 << width) as u128;
    Ok(
        json!({"hex":hex(result,1),"unsigned":result.to_string(),"delta":(new_base-old_base).to_string()}),
    )
}

fn packed_vertex_decode(args: &Value) -> Result<Value, String> {
    let format = input::string(args, "format")?;
    let raw = unsigned(input::integer(args, "pattern")?, 32)? as u32;
    let components = match format {
        "R10G10B10A2_UNORM" => vec![
            (raw & 0x3ff) as f64 / 1023.0,
            ((raw >> 10) & 0x3ff) as f64 / 1023.0,
            ((raw >> 20) & 0x3ff) as f64 / 1023.0,
            ((raw >> 30) & 3) as f64 / 3.0,
        ],
        "R8G8B8A8_UNORM" => (0..4)
            .map(|i| ((raw >> (i * 8)) & 0xff) as f64 / 255.0)
            .collect(),
        "R8G8B8A8_SNORM" => (0..4)
            .map(|i| {
                let signed = ((raw >> (i * 8)) & 0xff) as u8 as i8;
                (signed as f64 / 127.0).max(-1.0)
            })
            .collect(),
        "R16G16_FLOAT" => vec![
            half::f16::from_bits(raw as u16).to_f32() as f64,
            half::f16::from_bits((raw >> 16) as u16).to_f32() as f64,
        ],
        _ => return Err("Unsupported packed vertex format.".into()),
    };
    Ok(json!({"components":super::checked_array(components)?}))
}

fn crc_compute(args: &Value) -> Result<Value, String> {
    let data = bytes::parse_hex(input::string(args, "data")?)?;
    let width = input::int(args, "width")?;
    if !matches!(width, 8 | 16 | 32 | 64) {
        return Err("CRC width must be 8, 16, 32 or 64.".into());
    }
    let width = width as u32;
    let poly = unsigned(input::integer(args, "polynomial")?, width)?;
    let mut register = unsigned(input::integer(args, "init")?, width)?;
    let xor_out = unsigned(input::integer(args, "xorOut")?, width)?;
    let reflect_in = input::boolean(args, "reflectIn")?;
    let reflect_out = input::boolean(args, "reflectOut")?;
    let bit_mask = mask(width);
    if reflect_in {
        let reflected_poly = integer::reflect_bits(poly, width);
        for byte in data {
            register ^= u128::from(byte);
            for _ in 0..8 {
                register = if register & 1 != 0 {
                    (register >> 1) ^ reflected_poly
                } else {
                    register >> 1
                };
            }
        }
    } else {
        for byte in data {
            register ^= u128::from(byte) << (width - 8);
            for _ in 0..8 {
                register = if register & (1u128 << (width - 1)) != 0 {
                    ((register << 1) ^ poly) & bit_mask
                } else {
                    (register << 1) & bit_mask
                };
            }
        }
    }
    if reflect_out != reflect_in {
        register = integer::reflect_bits(register, width);
    }
    register = (register ^ xor_out) & bit_mask;
    Ok(json!({"hex":hex(register,(width/4) as usize),"unsigned":register.to_string()}))
}
