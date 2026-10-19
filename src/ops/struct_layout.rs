use crate::bytes::{self, ScalarType, format_hex, parse_hex};
use crate::input;
use serde_json::{Value, json};
use std::collections::HashMap;

const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_RECORDS: usize = 10_000;
const MAX_FIELDS: usize = 10_000;
const MAX_TYPE_BYTES: u64 = 1 << 40;
const MAX_DECODE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ARRAY_VALUES: u64 = 64;

#[derive(Clone, Copy, PartialEq)]
enum Abi {
    Win64,
    Win32,
    SysV64,
    SysV32,
}

impl Abi {
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "win64" | "msvc64" | "x64" => Ok(Self::Win64),
            "win32" | "msvc32" | "x86" => Ok(Self::Win32),
            "sysv64" | "linux64" | "gcc64" => Ok(Self::SysV64),
            "sysv32" | "linux32" | "gcc32" => Ok(Self::SysV32),
            _ => Err("'abi' must be win64, win32, sysv64 or sysv32.".into()),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Win64 => "win64",
            Self::Win32 => "win32",
            Self::SysV64 => "sysv64",
            Self::SysV32 => "sysv32",
        }
    }

    fn msvc(self) -> bool {
        matches!(self, Self::Win64 | Self::Win32)
    }

    fn pointer(self) -> u64 {
        if matches!(self, Self::Win64 | Self::SysV64) {
            8
        } else {
            4
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Num(u64),
    Punct(char),
    Pack(PackOp),
}

#[derive(Clone, Debug, PartialEq)]
enum PackOp {
    Set(Option<u64>),
    Push(Option<u64>),
    Pop,
}

#[derive(Clone)]
enum Base {
    Int { signed: bool },
    Float,
    Bool,
    Pointer,
    Record(usize),
    Opaque,
}

#[derive(Clone)]
struct Ty {
    base: Base,
    size: u64,
    align: u64,
    name: String,
    char_like: bool,
}

struct Member {
    name: Option<String>,
    ty: Ty,
    dims: Vec<u64>,
    bits: Option<u64>,
    align_override: Option<u64>,
}

struct Field {
    name: Option<String>,
    ty: Ty,
    dims: Vec<u64>,
    offset: u64,
    size: u64,
    align: u64,
    bitfield: Option<(u64, u64)>,
}

struct Record {
    name: String,
    union: bool,
    fields: Vec<Field>,
    size: u64,
    align: u64,
    pack: Option<u64>,
}

/// Tokenizes C source, expanding object-like `#define` macros as they are used.
fn tokenize(source: &str, macros: &mut HashMap<String, Vec<Tok>>) -> Result<Vec<Tok>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    let mut line_start = true;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == '#' && line_start {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            let line: String = chars[start + 1..i].iter().collect();
            directive(line.trim(), &mut tokens, macros)?;
            continue;
        }
        line_start = false;
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            match macros.get(&word) {
                Some(body) => tokens.extend(body.iter().cloned()),
                None => tokens.push(Tok::Ident(word)),
            }
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '\'') {
                i += 1;
            }
            let text: String = chars[start..i].iter().filter(|&&c| c != '\'').collect();
            tokens.push(Tok::Num(parse_number(&text)?));
        } else if "{}[]();:,*=+-/()<>&|~.".contains(c) {
            tokens.push(Tok::Punct(c));
            i += 1;
        } else {
            return Err(format!("Unexpected character '{c}' in C source."));
        }
    }
    if tokens.len() > MAX_SOURCE_BYTES {
        return Err("Macro expansion produced too many tokens.".into());
    }
    Ok(tokens)
}

fn parse_number(text: &str) -> Result<u64, String> {
    let lower = text.to_ascii_lowercase();
    let trimmed = lower.trim_end_matches(['u', 'l']);
    let parsed = if let Some(hex) = trimmed.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)
    } else if let Some(binary) = trimmed.strip_prefix("0b") {
        u64::from_str_radix(binary, 2)
    } else if trimmed.len() > 1 && trimmed.starts_with('0') {
        u64::from_str_radix(&trimmed[1..], 8)
    } else {
        trimmed.parse()
    };
    parsed.map_err(|_| format!("Invalid number '{text}'."))
}

fn directive(
    line: &str,
    tokens: &mut Vec<Tok>,
    macros: &mut HashMap<String, Vec<Tok>>,
) -> Result<(), String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.first().copied() {
        // Object-like macros only; function-like ones (NAME(...)) are ignored.
        Some("define") if words.len() >= 2 && !words[1].contains('(') => {
            let body = line
                .trim_start_matches("define")
                .trim_start()
                .strip_prefix(words[1])
                .unwrap_or("");
            let expansion = tokenize(body, macros)?;
            macros.insert(words[1].to_owned(), expansion);
        }
        Some("pragma") if line.contains("pack") => {
            let inner = line
                .split_once('(')
                .and_then(|(_, rest)| rest.rsplit_once(')'))
                .map(|(inner, _)| inner)
                .unwrap_or("");
            let parts: Vec<&str> = inner
                .split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .collect();
            let number = |text: &str| -> Result<u64, String> {
                let value = parse_number(text)?;
                if matches!(value, 1 | 2 | 4 | 8 | 16) {
                    Ok(value)
                } else {
                    Err("#pragma pack value must be 1, 2, 4, 8 or 16.".into())
                }
            };
            let op = match parts.as_slice() {
                [] => PackOp::Set(None),
                ["push"] => PackOp::Push(None),
                ["push", n] => PackOp::Push(Some(number(n)?)),
                ["pop"] | ["pop", ..] => PackOp::Pop,
                [n] => PackOp::Set(Some(number(n)?)),
                _ => return Err(format!("Unsupported pragma: #{line}")),
            };
            tokens.push(Tok::Pack(op));
        }
        _ => {}
    }
    Ok(())
}

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
    abi: Abi,
    pack: Option<u64>,
    pack_stack: Vec<Option<u64>>,
    constants: HashMap<String, u64>,
    records: Vec<Record>,
    tags: HashMap<String, usize>,
    typedefs: HashMap<String, (Ty, Vec<u64>)>,
}

/// A declarator's name, resulting type, array dimensions, bit width and attributes.
type Declarator = (Option<String>, Ty, Vec<u64>, Option<u64>, Attributes);

/// Attributes collected while reading a type specifier.
#[derive(Default)]
struct Attributes {
    packed: bool,
    aligned: Option<u64>,
}

fn align_up(value: u64, align: u64) -> u64 {
    value.div_ceil(align.max(1)) * align.max(1)
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn peek_ident(&self) -> Option<&str> {
        match self.peek() {
            Some(Tok::Ident(name)) => Some(name),
            _ => None,
        }
    }

    fn is_punct(&self, c: char) -> bool {
        self.peek() == Some(&Tok::Punct(c))
    }

    fn eat_punct(&mut self, c: char) -> bool {
        let matched = self.is_punct(c);
        if matched {
            self.pos += 1;
        }
        matched
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.eat_punct(c) {
            Ok(())
        } else {
            Err(format!("Expected '{c}' {}.", self.location()))
        }
    }

    fn location(&self) -> String {
        match self.peek() {
            Some(Tok::Ident(name)) => format!("before '{name}'"),
            Some(Tok::Num(n)) => format!("before '{n}'"),
            Some(Tok::Punct(c)) => format!("before '{c}'"),
            Some(Tok::Pack(_)) => "before #pragma pack".into(),
            None => "at end of input".into(),
        }
    }

    fn ident(&mut self) -> Option<String> {
        let name = self.peek_ident()?.to_owned();
        self.pos += 1;
        Some(name)
    }

    fn apply_pack(&mut self, op: PackOp) {
        match op {
            PackOp::Set(value) => self.pack = value,
            PackOp::Push(value) => {
                self.pack_stack.push(self.pack);
                if value.is_some() {
                    self.pack = value;
                }
            }
            PackOp::Pop => self.pack = self.pack_stack.pop().unwrap_or(None),
        }
    }

    fn take_pragmas(&mut self) {
        while let Some(Tok::Pack(op)) = self.peek().cloned() {
            self.pos += 1;
            self.apply_pack(op);
        }
    }

    // Constant expressions: + - * / << >> | & ~, parentheses, numbers, defines, enum values, sizeof.
    fn expr(&mut self) -> Result<u64, String> {
        self.binary(0)
    }

    fn binary(&mut self, level: u8) -> Result<u64, String> {
        const LEVELS: &[&[&str]] = &[&["|"], &["&"], &["<<", ">>"], &["+", "-"], &["*", "/"]];
        if level as usize == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        loop {
            let op = LEVELS[level as usize]
                .iter()
                .copied()
                .find(|op| self.at_operator(op));
            let Some(op) = op else { break };
            self.pos += op.len();
            let right = self.binary(level + 1)?;
            left = match op {
                "|" => left | right,
                "&" => left & right,
                "<<" => left.checked_shl(right as u32).ok_or("Shift overflows.")?,
                ">>" => left.checked_shr(right as u32).unwrap_or(0),
                "+" => left.checked_add(right).ok_or("Constant overflows.")?,
                "-" => left.checked_sub(right).ok_or("Constant underflows.")?,
                "*" => left.checked_mul(right).ok_or("Constant overflows.")?,
                "/" => left
                    .checked_div(right)
                    .ok_or("Division by zero in constant.")?,
                _ => unreachable!(),
            };
        }
        Ok(left)
    }

    fn at_operator(&self, op: &str) -> bool {
        op.chars()
            .enumerate()
            .all(|(i, c)| self.tokens.get(self.pos + i) == Some(&Tok::Punct(c)))
    }

    fn unary(&mut self) -> Result<u64, String> {
        if self.eat_punct('(') {
            let value = self.expr()?;
            self.expect(')')?;
            return Ok(value);
        }
        if self.eat_punct('~') {
            return Ok(!self.unary()?);
        }
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.pos += 1;
                Ok(n)
            }
            Some(Tok::Ident(name)) if name == "sizeof" => {
                self.pos += 1;
                self.expect('(')?;
                let (ty, dims, _, _) = self.type_spec()?;
                let mut pointer = false;
                while self.eat_punct('*') {
                    pointer = true;
                }
                self.expect(')')?;
                Ok(if pointer {
                    self.abi.pointer()
                } else {
                    ty.size * dims.iter().product::<u64>()
                })
            }
            Some(Tok::Ident(name)) => {
                self.pos += 1;
                self.constants
                    .get(&name)
                    .copied()
                    .ok_or_else(|| format!("Unknown constant '{name}'."))
            }
            _ => Err(format!("Expected a constant {}.", self.location())),
        }
    }

    fn builtin(&self, words: &[String]) -> Option<Ty> {
        let ptr = self.abi.pointer();
        let has = |w: &str| words.iter().any(|x| x == w);
        let longs = words.iter().filter(|w| *w == "long").count();
        let unsigned = has("unsigned");
        let int = |size: u64, signed: bool, name: &str| Ty {
            base: Base::Int { signed },
            size,
            // i386 System V aligns 8-byte integers to 4 inside structures.
            align: if size == 8 && self.abi == Abi::SysV32 {
                4
            } else {
                size
            },
            name: name.to_owned(),
            char_like: false,
        };
        let float = |size: u64, align: u64, name: &str| Ty {
            base: if size <= 8 { Base::Float } else { Base::Opaque },
            size,
            align,
            name: name.to_owned(),
            char_like: false,
        };
        let joined = words.join(" ");
        if has("char") {
            let mut ty = int(1, !unsigned, &joined);
            // Plain and signed char arrays read as text; unsigned char arrays are data.
            ty.char_like = !unsigned;
            return Some(ty);
        }
        if has("float") {
            return Some(float(4, 4, "float"));
        }
        if has("double") {
            return Some(match (longs, self.abi) {
                (0, Abi::SysV32) => float(8, 4, "double"),
                (0, _) => float(8, 8, "double"),
                (_, Abi::Win64 | Abi::Win32) => float(8, 8, "long double"),
                (_, Abi::SysV64) => float(16, 16, "long double"),
                (_, Abi::SysV32) => float(12, 4, "long double"),
            });
        }
        if has("short") {
            return Some(int(2, !unsigned, &joined));
        }
        if longs >= 2 {
            return Some(int(8, !unsigned, &joined));
        }
        if longs == 1 {
            let size = if self.abi == Abi::SysV64 { 8 } else { 4 };
            return Some(int(size, !unsigned, &joined));
        }
        if has("int") || has("signed") || unsigned {
            return Some(int(4, !unsigned, &joined));
        }
        let word = words.first()?.as_str();
        let wchar = if self.abi.msvc() { 2 } else { 4 };
        let (size, signed) = match word {
            "bool" | "_Bool" | "BOOLEAN" => {
                return Some(Ty {
                    base: Base::Bool,
                    size: 1,
                    align: 1,
                    name: word.to_owned(),
                    char_like: false,
                });
            }
            "int8_t" | "__int8" | "INT8" | "CHAR" | "CCHAR" | "i8" | "s8" => (1, true),
            "uint8_t" | "BYTE" | "UCHAR" | "UINT8" | "u8" => (1, false),
            "int16_t" | "__int16" | "INT16" | "SHORT" | "i16" | "s16" => (2, true),
            "uint16_t" | "WORD" | "USHORT" | "UINT16" | "char16_t" | "WCHAR" | "u16" => (2, false),
            "int32_t" | "__int32" | "INT32" | "INT" | "LONG" | "BOOL" | "HRESULT" | "i32"
            | "s32" => (4, true),
            "uint32_t" | "DWORD" | "UINT" | "UINT32" | "ULONG" | "char32_t" | "u32" => (4, false),
            "int64_t" | "__int64" | "INT64" | "LONGLONG" | "LONG64" | "i64" | "s64" => (8, true),
            "uint64_t" | "QWORD" | "UINT64" | "ULONGLONG" | "DWORD64" | "ULONG64" | "u64" => {
                (8, false)
            }
            "wchar_t" => (wchar, !self.abi.msvc()),
            "intptr_t" | "ptrdiff_t" | "ssize_t" | "INT_PTR" | "LONG_PTR" | "SSIZE_T" => {
                (ptr, true)
            }
            "size_t" | "uintptr_t" | "UINT_PTR" | "ULONG_PTR" | "DWORD_PTR" | "SIZE_T" => {
                (ptr, false)
            }
            "HANDLE" | "HMODULE" | "HINSTANCE" | "HWND" | "PVOID" | "LPVOID" | "LPCVOID"
            | "LPSTR" | "LPCSTR" | "LPWSTR" | "LPCWSTR" | "PCHAR" | "PBYTE" | "FARPROC" => {
                return Some(self.pointer_ty(word));
            }
            "float" | "FLOAT" | "f32" => return Some(float(4, 4, word)),
            "f64" => return Some(float(8, if self.abi == Abi::SysV32 { 4 } else { 8 }, word)),
            _ => return None,
        };
        let mut ty = int(size, signed, word);
        ty.char_like = matches!(word, "CHAR" | "char16_t" | "WCHAR" | "wchar_t" | "char32_t");
        Some(ty)
    }

    fn pointer_ty(&self, name: &str) -> Ty {
        Ty {
            base: Base::Pointer,
            size: self.abi.pointer(),
            align: if self.abi == Abi::SysV32 || self.abi == Abi::Win32 {
                4
            } else {
                8
            },
            name: name.to_owned(),
            char_like: false,
        }
    }

    /// Skips a balanced parenthesised group, returning the tokens inside.
    fn group(&mut self) -> Result<Vec<Tok>, String> {
        self.expect('(')?;
        let start = self.pos;
        let mut depth = 1;
        while depth > 0 {
            match self.peek() {
                Some(Tok::Punct('(')) => depth += 1,
                Some(Tok::Punct(')')) => depth -= 1,
                None => return Err("Unbalanced parentheses.".into()),
                _ => {}
            }
            self.pos += 1;
        }
        Ok(self.tokens[start..self.pos - 1].to_vec())
    }

    fn attribute(&mut self, attributes: &mut Attributes) -> Result<bool, String> {
        let Some(word) = self.peek_ident().map(str::to_owned) else {
            return Ok(false);
        };
        match word.as_str() {
            "alignas" | "_Alignas" => {
                self.pos += 1;
                self.expect('(')?;
                let value = self.expr()?;
                self.expect(')')?;
                attributes.aligned = Some(attributes.aligned.unwrap_or(1).max(value));
            }
            "__declspec" | "__attribute__" | "__attribute" => {
                self.pos += 1;
                let inner = self.group()?;
                for (i, token) in inner.iter().enumerate() {
                    match token {
                        Tok::Ident(name) if name == "packed" || name == "__packed__" => {
                            attributes.packed = true
                        }
                        Tok::Ident(name)
                            if name == "align" || name == "aligned" || name == "__aligned__" =>
                        {
                            let value = match inner.get(i + 2) {
                                Some(Tok::Num(n)) if inner.get(i + 1) == Some(&Tok::Punct('(')) => {
                                    *n
                                }
                                _ => 16,
                            };
                            attributes.aligned = Some(attributes.aligned.unwrap_or(1).max(value));
                        }
                        _ => {}
                    }
                }
            }
            _ => return Ok(false),
        }
        if let Some(value) = attributes.aligned
            && !value.is_power_of_two()
        {
            return Err("Alignment must be a power of two.".into());
        }
        Ok(true)
    }

    /// Reads a type specifier. Returns the type, array dimensions from a typedef, and
    /// whether it defined an anonymous struct or union inline.
    fn type_spec(&mut self) -> Result<(Ty, Vec<u64>, Attributes, bool), String> {
        let mut words: Vec<String> = Vec::new();
        let mut found: Option<(Ty, Vec<u64>)> = None;
        let mut attributes = Attributes::default();
        let mut anonymous = false;
        loop {
            if self.attribute(&mut attributes)? {
                continue;
            }
            let Some(word) = self.peek_ident().map(str::to_owned) else {
                break;
            };
            match word.as_str() {
                "const" | "volatile" | "static" | "mutable" | "register" | "restrict"
                | "__restrict" | "__unaligned" | "__ptr32" | "__ptr64" | "typename" | "class" => {
                    self.pos += 1;
                }
                "signed" | "unsigned" | "short" | "long" | "int" | "char" | "float" | "double"
                | "void"
                    if found.is_none() =>
                {
                    self.pos += 1;
                    words.push(word);
                }
                "struct" | "union" if found.is_none() && words.is_empty() => {
                    self.pos += 1;
                    let union = word == "union";
                    let mut inner = Attributes::default();
                    while self.attribute(&mut inner)? {}
                    let tag = self.ident();
                    while self.attribute(&mut inner)? {}
                    if self.is_punct('{') {
                        anonymous = tag.is_none();
                        let index = self.record_body(tag.clone(), union, &inner)?;
                        found = Some((self.record_ty(index), Vec::new()));
                    } else {
                        let tag = tag.ok_or("Expected a struct or union name.")?;
                        found = Some(match self.tags.get(&tag) {
                            Some(&index) => (self.record_ty(index), Vec::new()),
                            None => (
                                Ty {
                                    base: Base::Opaque,
                                    size: 0,
                                    align: 1,
                                    name: format!("{word} {tag} (incomplete)"),
                                    char_like: false,
                                },
                                Vec::new(),
                            ),
                        });
                    }
                }
                "enum" if found.is_none() && words.is_empty() => {
                    self.pos += 1;
                    found = Some((self.enum_spec()?, Vec::new()));
                }
                _ if found.is_none() && words.is_empty() => {
                    if let Some(typedef) = self.typedefs.get(&word).cloned() {
                        self.pos += 1;
                        found = Some(typedef);
                    } else if let Some(ty) = self.builtin(std::slice::from_ref(&word)) {
                        self.pos += 1;
                        found = Some((ty, Vec::new()));
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        let (ty, dims) = match (found, words.is_empty()) {
            (Some(found), _) => found,
            (None, false) if words.iter().all(|w| w == "void") => (
                Ty {
                    base: Base::Opaque,
                    size: 0,
                    align: 1,
                    name: "void".into(),
                    char_like: false,
                },
                Vec::new(),
            ),
            (None, false) => (self.builtin(&words).ok_or("Unsupported type.")?, Vec::new()),
            (None, true) => return Err(format!("Expected a type {}.", self.location())),
        };
        Ok((ty, dims, attributes, anonymous))
    }

    fn enum_spec(&mut self) -> Result<Ty, String> {
        let tag = self.ident();
        let mut ty = Ty {
            base: Base::Int { signed: true },
            size: 4,
            align: 4,
            name: format!("enum {}", tag.clone().unwrap_or_default())
                .trim()
                .to_owned(),
            char_like: false,
        };
        if self.eat_punct(':') {
            let (underlying, _, _, _) = self.type_spec()?;
            ty.base = underlying.base;
            ty.size = underlying.size;
            ty.align = underlying.align;
        }
        if self.eat_punct('{') {
            let mut next = 0u64;
            while !self.eat_punct('}') {
                let name = self.ident().ok_or("Expected an enumerator name.")?;
                if self.eat_punct('=') {
                    let negative = self.eat_punct('-');
                    let value = self.expr()?;
                    next = if negative {
                        value.wrapping_neg()
                    } else {
                        value
                    };
                }
                self.constants.insert(name, next);
                next = next.wrapping_add(1);
                if !self.eat_punct(',') {
                    self.expect('}')?;
                    break;
                }
            }
        }
        if let Some(tag) = tag {
            self.typedefs
                .insert(format!("enum {tag}"), (ty.clone(), Vec::new()));
        }
        Ok(ty)
    }

    fn record_ty(&self, index: usize) -> Ty {
        let record = &self.records[index];
        Ty {
            base: Base::Record(index),
            size: record.size,
            align: record.align,
            name: format!(
                "{} {}",
                if record.union { "union" } else { "struct" },
                record.name
            ),
            char_like: false,
        }
    }

    /// Reads one declarator: pointers, a name (optional), array dimensions and a bit width.
    fn declarator(&mut self, ty: &Ty, dims: &[u64]) -> Result<Declarator, String> {
        let mut attributes = Attributes::default();
        let mut pointer = false;
        loop {
            if self.eat_punct('*') || self.eat_punct('&') {
                pointer = true;
            } else if matches!(
                self.peek_ident(),
                Some("const" | "volatile" | "restrict" | "__restrict" | "__ptr32" | "__ptr64")
            ) {
                self.pos += 1;
            } else if !self.attribute(&mut attributes)? {
                break;
            }
        }
        let mut name = None;
        if self.is_punct('(') && self.tokens.get(self.pos + 1) == Some(&Tok::Punct('*')) {
            // Function pointer: ( * name ) ( parameters )
            self.pos += 2;
            while matches!(
                self.peek_ident(),
                Some("const" | "__cdecl" | "__stdcall" | "__fastcall" | "__thiscall")
            ) {
                self.pos += 1;
            }
            name = self.ident();
            self.expect(')')?;
            self.group()?;
            pointer = true;
        } else if let Some(ident) = self.peek_ident().map(str::to_owned) {
            self.pos += 1;
            name = Some(ident);
        }
        let mut own_dims = Vec::new();
        while self.eat_punct('[') {
            own_dims.push(if self.is_punct(']') { 0 } else { self.expr()? });
            self.expect(']')?;
        }
        while self.attribute(&mut attributes)? {}
        let bits = if self.eat_punct(':') {
            Some(self.expr()?)
        } else {
            None
        };
        while self.attribute(&mut attributes)? {}
        let (ty, mut all_dims) = if pointer {
            let mut p = self.pointer_ty(&format!("{}*", ty.name));
            p.name = format!("{} *", ty.name);
            (p, Vec::new())
        } else {
            (ty.clone(), dims.to_vec())
        };
        // Declared dimensions are outermost; typedef dimensions apply to each element.
        own_dims.append(&mut all_dims);
        Ok((name, ty, own_dims, bits, attributes))
    }

    fn record_body(
        &mut self,
        tag: Option<String>,
        union: bool,
        attributes: &Attributes,
    ) -> Result<usize, String> {
        self.expect('{')?;
        let mut members = Vec::new();
        loop {
            self.take_pragmas();
            if self.eat_punct('}') {
                break;
            }
            if self.peek().is_none() {
                return Err("Unterminated struct or union body.".into());
            }
            let (ty, dims, spec_attributes, anonymous) = self.type_spec()?;
            let align_override = spec_attributes.aligned;
            if self.eat_punct(';') {
                // Anonymous struct or union member: its fields belong to the parent.
                if anonymous {
                    members.push(Member {
                        name: None,
                        ty,
                        dims,
                        bits: None,
                        align_override,
                    });
                }
                continue;
            }
            loop {
                let (name, ty, dims, bits, declared) = self.declarator(&ty, &dims)?;
                if name.is_none() && bits.is_none() {
                    return Err(format!("Expected a member name {}.", self.location()));
                }
                let align_override = match (align_override, declared.aligned) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, b) => a.or(b),
                };
                members.push(Member {
                    name,
                    ty,
                    dims,
                    bits,
                    align_override,
                });
                if !self.eat_punct(',') {
                    break;
                }
            }
            self.expect(';')?;
        }
        let mut post = Attributes::default();
        while self.attribute(&mut post)? {}
        let pack = if attributes.packed || post.packed {
            Some(1)
        } else {
            self.pack
        };
        let record_align = attributes.aligned.max(post.aligned);
        let name = tag
            .clone()
            .unwrap_or_else(|| format!("<anonymous#{}>", self.records.len()));
        let record = layout(name, union, members, pack, record_align, self.abi)?;
        if self.records.len() >= MAX_RECORDS {
            return Err("Too many struct and union definitions.".into());
        }
        self.records.push(record);
        let index = self.records.len() - 1;
        if let Some(tag) = tag {
            self.tags.insert(tag, index);
        }
        Ok(index)
    }

    fn top_level(&mut self) -> Result<(), String> {
        loop {
            self.take_pragmas();
            if self.peek().is_none() {
                return Ok(());
            }
            if self.eat_punct(';') {
                continue;
            }
            let typedef = self.peek_ident() == Some("typedef");
            if typedef {
                self.pos += 1;
            }
            let (ty, dims, _, _) = self.type_spec()?;
            if self.eat_punct(';') {
                continue;
            }
            loop {
                let (name, ty, dims, _, _) = self.declarator(&ty, &dims)?;
                if typedef {
                    let name = name.ok_or("Expected a typedef name.")?;
                    if let Base::Record(index) = ty.base
                        && self.records[index].name.starts_with("<anonymous")
                        && dims.is_empty()
                    {
                        self.records[index].name = name.clone();
                    }
                    let mut named = ty.clone();
                    if matches!(named.base, Base::Record(_)) && dims.is_empty() {
                        named.name = name.clone();
                    }
                    self.typedefs.insert(name, (named, dims));
                }
                if !self.eat_punct(',') {
                    break;
                }
            }
            self.expect(';')?;
        }
    }
}

fn layout(
    name: String,
    union: bool,
    members: Vec<Member>,
    pack: Option<u64>,
    record_align: Option<u64>,
    abi: Abi,
) -> Result<Record, String> {
    let mut fields = Vec::new();
    let mut bit = 0u64; // next free bit (structs)
    let mut size = 0u64; // unions
    let mut max_align = 1u64;
    let mut unit: Option<(u64, u64, u64)> = None; // MSVC: (start byte, unit size, used bits)
    for member in members {
        let count = member
            .dims
            .iter()
            .try_fold(1u64, |a, &d| a.checked_mul(d))
            .ok_or("Array size overflows.")?;
        let total = member
            .ty
            .size
            .checked_mul(count)
            .filter(|&s| s <= MAX_TYPE_BYTES)
            .ok_or("Type is too large.")?;
        if member.ty.size == 0 && !(member.dims.first() == Some(&0)) {
            return Err(format!(
                "Member '{}' has incomplete type {}.",
                member.name.as_deref().unwrap_or("?"),
                member.ty.name
            ));
        }
        // #pragma pack caps natural alignment; alignas/aligned can still raise it.
        let packed = pack.map_or(member.ty.align, |p| member.ty.align.min(p));
        let align = packed.max(member.align_override.unwrap_or(1));
        if let Some(width) = member.bits {
            if !matches!(member.ty.base, Base::Int { .. } | Base::Bool) || !member.dims.is_empty() {
                return Err("Bit-fields must have an integer, bool or enum type.".into());
            }
            let unit_bits = member.ty.size * 8;
            if width > unit_bits {
                return Err(format!("Bit-field width {width} exceeds its type."));
            }
            if union {
                max_align = max_align.max(align);
                size = size.max(member.ty.size);
                if width > 0 {
                    fields.push(Field {
                        name: member.name,
                        ty: member.ty,
                        dims: Vec::new(),
                        offset: 0,
                        size: unit_bits / 8,
                        align,
                        bitfield: Some((0, width)),
                    });
                }
                continue;
            }
            // (storage unit start byte, bit offset within it)
            let (offset, bit_offset) = if abi.msvc() {
                // MSVC: a run of same-sized bit-fields shares one storage unit.
                match unit {
                    // A zero-width bit-field after a bit-field closes the unit and
                    // contributes its type's alignment; after anything else it is ignored.
                    _ if width == 0 => {
                        if unit.take().is_some() {
                            bit = align_up(bit, align * 8);
                            max_align = max_align.max(align);
                        }
                        continue;
                    }
                    Some((start, unit_size, used))
                        if unit_size == member.ty.size && used + width <= unit_bits =>
                    {
                        unit = Some((start, unit_size, used + width));
                        (start, used)
                    }
                    _ => {
                        let start = align_up(bit.div_ceil(8), align);
                        unit = Some((start, member.ty.size, width));
                        bit = (start + member.ty.size) * 8;
                        max_align = max_align.max(align);
                        (start, 0)
                    }
                }
            } else {
                // System V: bit-fields pack at the next bit unless they would cross
                // an alignment boundary of their declared type.
                if width == 0 {
                    bit = align_up(bit, member.ty.align * 8);
                    continue;
                }
                // The field must fit in a sizeof(type) unit starting on an alignment
                // boundary (i386 long long: 64-bit units on any 32-bit boundary).
                let align_bits = align * 8;
                if pack.is_none() && (bit / align_bits) * align_bits + unit_bits < bit + width {
                    bit = align_up(bit, align_bits);
                }
                let position = bit;
                bit += width;
                if member.name.is_some() {
                    max_align = max_align.max(align);
                }
                // Report the storage unit containing the field when it fits in one.
                let unit_start = (position / 8 / member.ty.size) * member.ty.size;
                let offset = if position + width <= (unit_start + member.ty.size) * 8 {
                    unit_start
                } else {
                    position / 8
                };
                (offset, position - offset * 8)
            };
            fields.push(Field {
                name: member.name,
                ty: member.ty.clone(),
                dims: Vec::new(),
                offset,
                size: member.ty.size,
                align,
                bitfield: Some((bit_offset, width)),
            });
            continue;
        }
        unit = None;
        max_align = max_align.max(align);
        let offset = if union {
            0
        } else {
            align_up(bit.div_ceil(8), align)
        };
        if union {
            size = size.max(total);
        } else {
            bit = (offset + total) * 8;
        }
        fields.push(Field {
            name: member.name,
            ty: member.ty,
            dims: member.dims,
            offset,
            size: total,
            align,
            bitfield: None,
        });
        if fields.len() > MAX_FIELDS {
            return Err("Too many fields.".into());
        }
    }
    let mut align = match pack {
        Some(p) => max_align.min(p.max(1)).max(1),
        None => max_align,
    };
    if let Some(forced) = record_align {
        align = align.max(forced);
    }
    let used = if union { size } else { bit.div_ceil(8) };
    Ok(Record {
        name,
        union,
        fields,
        size: align_up(used, align),
        align,
        pack,
    })
}

struct Leaf<'a> {
    path: String,
    field: &'a Field,
    offset: u64,
}

fn type_name(field: &Field) -> String {
    let dims: String = field.dims.iter().map(|d| format!("[{d}]")).collect();
    format!("{}{dims}", field.ty.name)
}

fn flatten<'a>(
    records: &'a [Record],
    index: usize,
    base: u64,
    prefix: &str,
    depth: usize,
    rows: &mut Vec<Value>,
    leaves: &mut Vec<Leaf<'a>>,
) -> Result<(), String> {
    if depth > 64 {
        return Err("Structures are nested too deeply.".into());
    }
    for field in &records[index].fields {
        let offset = base + field.offset;
        let path = match &field.name {
            Some(name) => format!("{prefix}{name}"),
            None if matches!(field.ty.base, Base::Record(_)) && field.dims.is_empty() => {
                prefix.trim_end_matches('.').to_owned()
            }
            None => format!("{prefix}(unnamed)"),
        };
        let nested = match field.ty.base {
            Base::Record(child) if field.dims.is_empty() => Some(child),
            _ => None,
        };
        if field.name.is_some() || nested.is_none() {
            if rows.len() >= MAX_FIELDS {
                return Err("Too many fields to report.".into());
            }
            let mut row = json!({"name":path,"type":type_name(field),"offset":offset,"offsetHex":format!("0x{offset:X}"),
                "size":field.size,"align":field.align,"depth":depth});
            if !field.dims.is_empty() {
                row["count"] = json!(field.dims.iter().product::<u64>());
            }
            if let Some((bit, width)) = field.bitfield {
                row["bitOffset"] = json!(bit);
                row["bitWidth"] = json!(width);
            }
            rows.push(row);
        }
        match nested {
            Some(child) => {
                let child_prefix = if field.name.is_some() {
                    format!("{path}.")
                } else {
                    prefix.to_owned()
                };
                flatten(
                    records,
                    child,
                    offset,
                    &child_prefix,
                    depth + usize::from(field.name.is_some()),
                    rows,
                    leaves,
                )?;
            }
            None => leaves.push(Leaf {
                path,
                field,
                offset,
            }),
        }
    }
    Ok(())
}

fn padding(size: u64, leaves: &[Leaf]) -> Vec<Value> {
    let mut spans: Vec<(u64, u64)> = leaves
        .iter()
        .map(|leaf| (leaf.offset, leaf.offset + leaf.field.size))
        .collect();
    spans.sort();
    let mut gaps = Vec::new();
    let mut cursor = 0;
    for (start, end) in spans.into_iter().chain(std::iter::once((size, size))) {
        if start > cursor {
            gaps.push(
                json!({"offset":cursor,"offsetHex":format!("0x{cursor:X}"),"size":start - cursor}),
            );
        }
        cursor = cursor.max(end);
    }
    gaps
}

fn scalar(data: &[u8], offset: usize, ty: &Ty, little: bool) -> Result<Value, String> {
    let at = |kind| bytes::read_scalar(data, offset, kind, little);
    Ok(match ty.base {
        Base::Int { signed } if matches!(ty.size, 1 | 2 | 4 | 8) => at(if signed {
            ScalarType::Signed(ty.size as usize)
        } else {
            ScalarType::Unsigned(ty.size as usize)
        })?,
        Base::Float if matches!(ty.size, 4 | 8) => at(ScalarType::Float(ty.size as usize))?,
        Base::Bool => json!(data[offset] != 0),
        Base::Pointer => {
            let raw = at(ScalarType::Unsigned(ty.size as usize))?;
            let value: u64 = raw.as_str().unwrap().parse().unwrap();
            json!(format!("0x{value:X}"))
        }
        _ => json!(format_hex(&data[offset..offset + ty.size as usize])),
    })
}

fn decode(leaf: &Leaf, data: &[u8], little: bool) -> Result<Value, String> {
    let field = leaf.field;
    let offset = leaf.offset as usize;
    if let Some((bit, width)) = field.bitfield {
        let unit = &data[offset..offset + field.size as usize];
        let raw = unit
            .iter()
            .rev()
            .fold(0u128, |acc, &b| (acc << 8) | u128::from(b));
        let mask = if width == 128 {
            u128::MAX
        } else {
            (1u128 << width) - 1
        };
        let value = (raw >> bit) & mask;
        return Ok(match field.ty.base {
            Base::Int { signed: true } if width > 0 && value >> (width - 1) & 1 == 1 => {
                json!((value as i128 - (1i128 << width)).to_string())
            }
            Base::Bool => json!(value != 0),
            _ => json!(value.to_string()),
        });
    }
    if field.dims.is_empty() {
        return scalar(data, offset, &field.ty, little);
    }
    let count = field.dims.iter().product::<u64>();
    let element = field.ty.size as usize;
    if field.ty.char_like {
        let units: Vec<u32> = (0..count as usize)
            .map(|i| {
                let b = &data[offset + i * element..offset + (i + 1) * element];
                if little {
                    b.iter().rev().fold(0u32, |a, &x| (a << 8) | u32::from(x))
                } else {
                    b.iter().fold(0u32, |a, &x| (a << 8) | u32::from(x))
                }
            })
            .take_while(|&u| u != 0)
            .collect();
        let text = match element {
            1 => String::from_utf8_lossy(&units.iter().map(|&u| u as u8).collect::<Vec<_>>())
                .into_owned(),
            2 => String::from_utf16_lossy(&units.iter().map(|&u| u as u16).collect::<Vec<_>>()),
            _ => units.iter().filter_map(|&u| char::from_u32(u)).collect(),
        };
        return Ok(json!(text));
    }
    if matches!(field.ty.base, Base::Record(_) | Base::Opaque) || element == 1 {
        let shown = (field.size as usize).min(64);
        return Ok(json!(format_hex(&data[offset..offset + shown])));
    }
    (0..count.min(MAX_ARRAY_VALUES) as usize)
        .map(|i| scalar(data, offset + i * element, &field.ty, little))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn read_data(args: &Value, size: u64) -> Result<Option<Vec<u8>>, String> {
    let data = match (args.get("data"), args.get("path")) {
        (None, None) => return Ok(None),
        (Some(_), None) => parse_hex(input::string(args, "data")?)?,
        (None, Some(_)) => {
            use std::io::{Read, Seek, SeekFrom};
            if size > MAX_DECODE_BYTES {
                return Err("Decoding is limited to structures of at most 16 MiB.".into());
            }
            let path = input::string(args, "path")?;
            let offset = match args.get("offset") {
                None => 0,
                Some(_) => u64::try_from(input::big_integer(args, "offset")?)
                    .map_err(|_| "'offset' must be a nonnegative integer.")?,
            };
            let mut file =
                std::fs::File::open(path).map_err(|e| format!("Cannot open file: {e}"))?;
            let mut buffer = vec![0; size as usize];
            file.seek(SeekFrom::Start(offset))
                .and_then(|_| file.read_exact(&mut buffer))
                .map_err(|_| format!("The file has fewer than {size} bytes at offset {offset}."))?;
            buffer
        }
        _ => return Err("Supply either 'data' or 'path', not both.".into()),
    };
    if (data.len() as u64) < size {
        return Err(format!(
            "'data' has {} bytes but the structure needs {size}.",
            data.len()
        ));
    }
    Ok(Some(data))
}

pub fn execute(args: &Value) -> Result<Value, String> {
    let source = input::string(args, "source")?;
    if source.len() > MAX_SOURCE_BYTES {
        return Err("'source' may be at most 1 MiB.".into());
    }
    let abi = Abi::parse(args.get("abi").and_then(Value::as_str).unwrap_or("win64"))?;
    let tokens = tokenize(source, &mut HashMap::new())?;
    let mut parser = Parser {
        tokens,
        pos: 0,
        abi,
        pack: None,
        pack_stack: Vec::new(),
        constants: HashMap::new(),
        records: Vec::new(),
        tags: HashMap::new(),
        typedefs: HashMap::new(),
    };
    if let Some(pack) = input::optional_int(args, "pack")? {
        if !matches!(pack, 1 | 2 | 4 | 8 | 16) {
            return Err("'pack' must be 1, 2, 4, 8 or 16.".into());
        }
        parser.pack = Some(pack as u64);
    }
    parser.top_level()?;
    let named: Vec<usize> = (0..parser.records.len())
        .filter(|&i| !parser.records[i].name.starts_with("<anonymous"))
        .collect();
    let index = match args.get("name").and_then(Value::as_str) {
        Some(name) => {
            let name = name
                .trim_start_matches("struct ")
                .trim_start_matches("union ");
            parser
                .tags
                .get(name)
                .copied()
                .or_else(|| match parser.typedefs.get(name) {
                    Some((
                        Ty {
                            base: Base::Record(i),
                            ..
                        },
                        dims,
                    )) if dims.is_empty() => Some(*i),
                    _ => None,
                })
                .ok_or_else(|| format!("No struct or union named '{name}'."))?
        }
        None => *named
            .last()
            .ok_or("The source defines no named struct or union.")?,
    };
    let records = &parser.records;
    let record = &records[index];
    let mut rows = Vec::new();
    let mut leaves = Vec::new();
    flatten(records, index, 0, "", 0, &mut rows, &mut leaves)?;
    let gaps = padding(record.size, &leaves);
    let padding_bytes: u64 = gaps.iter().map(|g| g["size"].as_u64().unwrap()).sum();
    let mut result = json!({
        "name":record.name,"kind":if record.union {"union"} else {"struct"},"abi":abi.name(),
        "pack":record.pack,"size":record.size,"sizeHex":format!("0x{:X}",record.size),"align":record.align,
        "fields":rows,"padding":gaps,"paddingBytes":padding_bytes,
        "records":named.iter().map(|&i| json!({"name":records[i].name,"size":records[i].size,"align":records[i].align})).collect::<Vec<_>>()
    });
    if let Some(data) = read_data(args, record.size)? {
        let little = bytes::endian(
            args.get("endian")
                .and_then(Value::as_str)
                .unwrap_or("little"),
        )?;
        let mut values = serde_json::Map::new();
        for leaf in &leaves {
            if leaf.field.name.is_none() {
                continue;
            }
            values.insert(leaf.path.clone(), decode(leaf, &data, little)?);
        }
        result["values"] = Value::Object(values);
    }
    Ok(result)
}
