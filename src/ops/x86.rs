use crate::bytes::{format_hex, parse_hex};
use crate::input;
use iced_x86::{
    BlockEncoder, BlockEncoderOptions, Decoder, DecoderOptions, FastFormatter, FlowControl,
    Formatter, GasFormatter, Instruction, InstructionBlock, IntelFormatter, MasmFormatter,
    NasmFormatter, OpKind,
};
use serde_json::{Value, json};

const MAX_CODE_BYTES: usize = 64 * 1024;
const MAX_INSTRUCTIONS: usize = 100_000;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "x86_disasm" => disasm(args),
        "x86_assemble" => assemble(args),
        "signature_generate" => signature_generate(args),
        _ => unreachable!(),
    }
}

fn bitness(args: &Value) -> Result<u32, String> {
    let bits = input::optional_int(args, "bits")?.unwrap_or(64);
    match bits {
        16 | 32 | 64 => Ok(bits as u32),
        _ => Err("'bits' must be 16, 32 or 64.".into()),
    }
}

fn parse_ip(args: &Value, key: &str, default: u64) -> Result<u64, String> {
    match args.get(key) {
        None => Ok(default),
        Some(_) => {
            let value = input::big_integer(args, key)?;
            u64::try_from(value).map_err(|_| format!("'{key}' must be an unsigned 64-bit integer."))
        }
    }
}

fn format_instruction(syntax: &str, instruction: &Instruction) -> Result<String, String> {
    let mut output = String::new();
    let mut formatter: Box<dyn Formatter> = match syntax {
        "intel" => Box::new(IntelFormatter::new()),
        "nasm" => Box::new(NasmFormatter::new()),
        "masm" => Box::new(MasmFormatter::new()),
        "gas" | "att" => Box::new(GasFormatter::new()),
        "fast" => {
            let mut fast = FastFormatter::new();
            fast.format(instruction, &mut output);
            return Ok(output);
        }
        _ => return Err("'syntax' must be intel, nasm, masm, gas or fast.".into()),
    };
    formatter.format(instruction, &mut output);
    Ok(output)
}

fn flow_name(flow: FlowControl) -> &'static str {
    match flow {
        FlowControl::Next => "next",
        FlowControl::UnconditionalBranch => "unconditional_branch",
        FlowControl::IndirectBranch => "indirect_branch",
        FlowControl::ConditionalBranch => "conditional_branch",
        FlowControl::Return => "return",
        FlowControl::Call => "call",
        FlowControl::IndirectCall => "indirect_call",
        FlowControl::Interrupt => "interrupt",
        FlowControl::XbeginXabortXend => "xbegin_xabort_xend",
        FlowControl::Exception => "exception",
    }
}

fn branch_target(instruction: &Instruction) -> Option<u64> {
    match instruction.op0_kind() {
        OpKind::NearBranch16 | OpKind::NearBranch32 | OpKind::NearBranch64 => {
            Some(instruction.near_branch_target())
        }
        _ => None,
    }
}

fn disasm(args: &Value) -> Result<Value, String> {
    let bits = bitness(args)?;
    let code = parse_hex(input::string(args, "code")?)?;
    if code.is_empty() {
        return Err("'code' must contain at least one byte.".into());
    }
    if code.len() > MAX_CODE_BYTES {
        return Err("'code' must be at most 64 KiB.".into());
    }
    let syntax = args.get("syntax").and_then(Value::as_str).unwrap_or("intel");
    let ip = parse_ip(args, "address", 0)?;
    let limit = input::optional_int(args, "limit")?.unwrap_or(1000).max(1) as usize;
    let mut decoder = Decoder::with_ip(bits, &code, ip, DecoderOptions::NONE);
    let mut instruction = Instruction::default();
    let mut rows = Vec::new();
    let mut invalid = 0usize;
    while decoder.can_decode() && rows.len() < limit {
        decoder.decode_out(&mut instruction);
        let start = (instruction.ip() - ip) as usize;
        let raw = &code[start..start + instruction.len()];
        let text = format_instruction(syntax, &instruction)?;
        if instruction.is_invalid() {
            invalid += 1;
        }
        let mut row = json!({"address":format!("0x{:x}",instruction.ip()),"offset":start,
            "bytes":format_hex(raw),"text":text,"length":instruction.len(),
            "mnemonic":format!("{:?}",instruction.mnemonic()).to_lowercase(),
            "flowControl":flow_name(instruction.flow_control())});
        if let Some(target) = branch_target(&instruction) {
            row["branchTarget"] = json!(format!("0x{target:x}"));
        }
        if instruction.is_ip_rel_memory_operand() {
            row["ripTarget"] = json!(format!("0x{:x}", instruction.ip_rel_memory_address()));
        }
        rows.push(row);
    }
    let decoded = decoder.position();
    Ok(json!({"bits":bits,"address":format!("0x{ip:x}"),"instructionCount":rows.len(),
        "instructions":rows,"invalidCount":invalid,"decodedBytes":decoded,
        "remainingBytes":code.len()-decoded}))
}

/// Re-encodes a block of machine code at a new base address, fixing up relative
/// branches and RIP-relative operands so the bytes run correctly once relocated.
fn assemble(args: &Value) -> Result<Value, String> {
    let bits = bitness(args)?;
    let code = parse_hex(input::string(args, "code")?)?;
    if code.is_empty() {
        return Err("'code' must contain at least one byte.".into());
    }
    if code.len() > MAX_CODE_BYTES {
        return Err("'code' must be at most 64 KiB.".into());
    }
    let source_ip = parse_ip(args, "sourceAddress", 0)?;
    let target_ip = parse_ip(args, "targetAddress", source_ip)?;
    let fix_branches = !input::optional_bool(args, "dontFixBranches").unwrap_or(false);
    let mut decoder = Decoder::with_ip(bits, &code, source_ip, DecoderOptions::NONE);
    let mut instructions = Vec::new();
    while decoder.can_decode() {
        let instruction = decoder.decode();
        if instruction.is_invalid() {
            return Err(format!(
                "Byte stream did not decode to valid instructions at offset 0x{:x}.",
                instruction.ip() - source_ip
            ));
        }
        instructions.push(instruction);
        if instructions.len() > MAX_INSTRUCTIONS {
            return Err("Too many instructions to assemble.".into());
        }
    }
    let options = if fix_branches {
        BlockEncoderOptions::NONE
    } else {
        BlockEncoderOptions::DONT_FIX_BRANCHES
    };
    let block = InstructionBlock::new(&instructions, target_ip);
    let result = BlockEncoder::encode(bits, block, options)
        .map_err(|e| format!("Encoding failed: {e}"))?;
    Ok(json!({"bits":bits,"sourceAddress":format!("0x{source_ip:x}"),
        "targetAddress":format!("0x{target_ip:x}"),"instructionCount":instructions.len(),
        "bytes":format_hex(&result.code_buffer),"byteCount":result.code_buffer.len(),
        "rewritten":result.code_buffer != code && target_ip != source_ip}))
}

/// Produces the shortest wildcarded signature that uniquely matches the bytes at
/// `offset`/`rva`, wildcarding relative and RIP-relative operand bytes so the
/// pattern survives rebasing and nearby edits.
fn signature_generate(args: &Value) -> Result<Value, String> {
    let bits = bitness(args)?;
    let code = parse_hex(input::string(args, "code")?)?;
    if code.is_empty() {
        return Err("'code' must contain at least one byte.".into());
    }
    if code.len() > MAX_CODE_BYTES {
        return Err("'code' must be at most 64 KiB.".into());
    }
    let start = input::optional_int(args, "offset")?.unwrap_or(0);
    if start < 0 || start as usize >= code.len() {
        return Err("'offset' must index a byte within 'code'.".into());
    }
    let start = start as usize;
    let min_instructions = input::optional_int(args, "minInstructions")?.unwrap_or(1).max(1) as usize;
    let wildcard_operands = !input::optional_bool(args, "exactOperands").unwrap_or(false);
    let base_ip = parse_ip(args, "address", 0)?;
    // Decode from the signature start; mark operand bytes that depend on placement.
    let region = &code[start..];
    let mut decoder = Decoder::with_ip(bits, region, base_ip.wrapping_add(start as u64), DecoderOptions::NONE);
    let mut instruction = Instruction::default();
    // Each pattern byte: Some(b) fixed, None wildcard.
    let mut pattern: Vec<Option<u8>> = Vec::new();
    let mut instruction_count = 0usize;
    while decoder.can_decode() {
        decoder.decode_out(&mut instruction);
        if instruction.is_invalid() {
            break;
        }
        let offsets = decoder.get_constant_offsets(&instruction);
        let length = instruction.len();
        let instruction_start = pattern.len();
        for i in 0..length {
            pattern.push(Some(region[instruction_start + i]));
        }
        if wildcard_operands {
            let wildcard_relative = matches!(
                instruction.flow_control(),
                FlowControl::ConditionalBranch
                    | FlowControl::UnconditionalBranch
                    | FlowControl::Call
            ) && branch_target(&instruction).is_some();
            if (offsets.has_displacement() && instruction.is_ip_rel_memory_operand())
                || (wildcard_relative && offsets.has_displacement())
            {
                for i in 0..offsets.displacement_size() {
                    pattern[instruction_start + offsets.displacement_offset() + i] = None;
                }
            }
            if wildcard_relative && offsets.has_immediate() {
                for i in 0..offsets.immediate_size() {
                    pattern[instruction_start + offsets.immediate_offset() + i] = None;
                }
            }
        }
        instruction_count += 1;
        // Grow until the pattern is unique in the whole image and covers the minimum.
        if instruction_count >= min_instructions && is_unique(&code, &pattern) {
            break;
        }
    }
    if pattern.is_empty() {
        return Err("Could not decode any instruction at the offset.".into());
    }
    // Trim trailing wildcards; they add nothing to a signature.
    while matches!(pattern.last(), Some(None)) {
        pattern.pop();
    }
    let unique = is_unique(&code, &pattern);
    let ida = pattern
        .iter()
        .map(|b| b.map_or("?".to_owned(), |v| format!("{v:02X}")))
        .collect::<Vec<_>>()
        .join(" ");
    let code_style = pattern
        .iter()
        .map(|b| b.map_or("??".to_owned(), |v| format!("{v:02X}")))
        .collect::<Vec<_>>()
        .join(" ");
    let mask: String = pattern
        .iter()
        .map(|b| if b.is_some() { 'x' } else { '?' })
        .collect();
    Ok(json!({"bits":bits,"offset":start,"instructionCount":instruction_count,
        "byteLength":pattern.len(),"wildcardCount":pattern.iter().filter(|b| b.is_none()).count(),
        "unique":unique,"pattern":code_style,"idaPattern":ida,"mask":mask}))
}

fn matches_at(code: &[u8], pattern: &[Option<u8>], position: usize) -> bool {
    pattern
        .iter()
        .enumerate()
        .all(|(i, byte)| byte.is_none_or(|v| code[position + i] == v))
}

fn is_unique(code: &[u8], pattern: &[Option<u8>]) -> bool {
    if pattern.len() > code.len() {
        return false;
    }
    let mut hits = 0;
    for position in 0..=code.len() - pattern.len() {
        if matches_at(code, pattern, position) {
            hits += 1;
            if hits > 1 {
                return false;
            }
        }
    }
    hits == 1
}
