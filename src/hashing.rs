//! Non-cryptographic hashes used by file formats and game engines.

pub const ALGORITHMS: &[&str] = &[
    "joaat",
    "fnv1_32",
    "fnv1a_32",
    "fnv1_64",
    "fnv1a_64",
    "murmur3_32",
    "xxh32",
    "xxh64",
    "xxh3_64",
    "djb2",
    "djb2a",
    "sdbm",
];

#[derive(Clone, Copy)]
pub enum Algorithm {
    Joaat,
    Fnv1_32,
    Fnv1a32,
    Fnv1_64,
    Fnv1a64,
    Murmur3_32,
    Xxh32,
    Xxh64,
    Xxh3_64,
    Djb2,
    Djb2a,
    Sdbm,
}

impl Algorithm {
    pub fn parse(name: &str) -> Result<Self, String> {
        Ok(match name {
            "joaat" => Self::Joaat,
            "fnv1_32" => Self::Fnv1_32,
            "fnv1a_32" => Self::Fnv1a32,
            "fnv1_64" => Self::Fnv1_64,
            "fnv1a_64" => Self::Fnv1a64,
            "murmur3_32" => Self::Murmur3_32,
            "xxh32" => Self::Xxh32,
            "xxh64" => Self::Xxh64,
            "xxh3_64" => Self::Xxh3_64,
            "djb2" => Self::Djb2,
            "djb2a" => Self::Djb2a,
            "sdbm" => Self::Sdbm,
            _ => {
                return Err(format!(
                    "'algorithm' must be one of {}.",
                    ALGORITHMS.join(", ")
                ));
            }
        })
    }

    pub fn bits(self) -> u32 {
        match self {
            Self::Fnv1_64 | Self::Fnv1a64 | Self::Xxh64 | Self::Xxh3_64 => 64,
            _ => 32,
        }
    }

    /// Seeds apply to the Murmur and xxHash families and replace the initial
    /// state of JOAAT, djb2 and sdbm. FNV uses its standard offset basis.
    pub fn seeded(self) -> bool {
        !matches!(
            self,
            Self::Fnv1_32 | Self::Fnv1a32 | Self::Fnv1_64 | Self::Fnv1a64
        )
    }

    pub fn seed_bits(self) -> u32 {
        match self {
            Self::Xxh64 | Self::Xxh3_64 => 64,
            _ => 32,
        }
    }

    pub fn default_seed(self) -> u64 {
        match self {
            Self::Djb2 | Self::Djb2a => 5381,
            _ => 0,
        }
    }

    pub fn hash(self, data: &[u8], seed: u64) -> u64 {
        match self {
            Self::Joaat => joaat(data, seed as u32) as u64,
            Self::Fnv1_32 => fnv32(data, false) as u64,
            Self::Fnv1a32 => fnv32(data, true) as u64,
            Self::Fnv1_64 => fnv64(data, false),
            Self::Fnv1a64 => fnv64(data, true),
            Self::Murmur3_32 => murmur3_32(data, seed as u32) as u64,
            Self::Xxh32 => xxhash_rust::xxh32::xxh32(data, seed as u32) as u64,
            Self::Xxh64 => xxhash_rust::xxh64::xxh64(data, seed),
            Self::Xxh3_64 => xxhash_rust::xxh3::xxh3_64_with_seed(data, seed),
            Self::Djb2 => data.iter().fold(seed as u32, |h, &b| {
                h.wrapping_mul(33).wrapping_add(b as u32)
            }) as u64,
            Self::Djb2a => {
                data.iter()
                    .fold(seed as u32, |h, &b| h.wrapping_mul(33) ^ b as u32) as u64
            }
            Self::Sdbm => data.iter().fold(seed as u32, |h, &b| {
                (b as u32)
                    .wrapping_add(h << 6)
                    .wrapping_add(h << 16)
                    .wrapping_sub(h)
            }) as u64,
        }
    }
}

fn joaat(data: &[u8], seed: u32) -> u32 {
    let mut hash = seed;
    for &byte in data {
        hash = hash.wrapping_add(byte as u32);
        hash = hash.wrapping_add(hash << 10);
        hash ^= hash >> 6;
    }
    hash = hash.wrapping_add(hash << 3);
    hash ^= hash >> 11;
    hash.wrapping_add(hash << 15)
}

fn fnv32(data: &[u8], alternate: bool) -> u32 {
    data.iter().fold(0x811c_9dc5, |hash: u32, &byte| {
        if alternate {
            (hash ^ byte as u32).wrapping_mul(0x0100_0193)
        } else {
            hash.wrapping_mul(0x0100_0193) ^ byte as u32
        }
    })
}

fn fnv64(data: &[u8], alternate: bool) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |hash: u64, &byte| {
        if alternate {
            (hash ^ byte as u64).wrapping_mul(0x0100_0000_01b3)
        } else {
            hash.wrapping_mul(0x0100_0000_01b3) ^ byte as u64
        }
    })
}

fn murmur3_32(data: &[u8], seed: u32) -> u32 {
    const C1: u32 = 0xcc9e_2d51;
    const C2: u32 = 0x1b87_3593;
    let mix = |k: u32| k.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
    let (blocks, tail) = data.as_chunks::<4>();
    let mut hash = seed;
    for block in blocks {
        hash ^= mix(u32::from_le_bytes(*block));
        hash = hash
            .rotate_left(13)
            .wrapping_mul(5)
            .wrapping_add(0xe654_6b64);
    }
    if !tail.is_empty() {
        let k = tail
            .iter()
            .rev()
            .fold(0u32, |k, &byte| (k << 8) | byte as u32);
        hash ^= mix(k);
    }
    hash ^= data.len() as u32;
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x85eb_ca6b);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(0xc2b2_ae35);
    hash ^ (hash >> 16)
}

#[derive(Clone, Copy, PartialEq)]
pub enum CaseFold {
    None,
    Lower,
    Upper,
}

impl CaseFold {
    pub fn parse(name: Option<&str>) -> Result<Self, String> {
        match name.unwrap_or("none") {
            "none" => Ok(Self::None),
            "lower" => Ok(Self::Lower),
            "upper" => Ok(Self::Upper),
            _ => Err("'caseFold' must be none, lower or upper.".into()),
        }
    }

    /// Folds ASCII letters only, which is what engine hash tables do.
    pub fn apply(self, bytes: &mut [u8]) {
        match self {
            Self::None => {}
            Self::Lower => bytes.make_ascii_lowercase(),
            Self::Upper => bytes.make_ascii_uppercase(),
        }
    }
}

pub fn encode_text(text: &str, encoding: &str) -> Result<Vec<u8>, String> {
    match encoding {
        "utf8" => Ok(text.as_bytes().to_vec()),
        "utf16le" => Ok(text.encode_utf16().flat_map(u16::to_le_bytes).collect()),
        "hex" => crate::bytes::parse_hex(text),
        _ => Err("'encoding' must be utf8, utf16le or hex.".into()),
    }
}

pub fn describe(value: u64, bits: u32) -> serde_json::Value {
    let digits = (bits / 4) as usize;
    let signed = if bits == 32 {
        (value as u32 as i32) as i64
    } else {
        value as i64
    };
    serde_json::json!({"hex":format!("0x{value:0digits$X}"),"unsigned":value.to_string(),"signed":signed.to_string()})
}

#[cfg(test)]
mod tests {
    use super::Algorithm::*;

    #[test]
    fn known_vectors() {
        let fox = b"The quick brown fox jumps over the lazy dog";
        assert_eq!(Joaat.hash(b"a", 0), 0xca2e_9442);
        assert_eq!(Joaat.hash(fox, 0), 0x519e_91f5);
        assert_eq!(Fnv1_32.hash(b"a", 0), 0x050c_5d7e);
        assert_eq!(Fnv1a32.hash(b"a", 0), 0xe40c_292c);
        assert_eq!(Fnv1_64.hash(b"a", 0), 0xaf63_bd4c_8601_b7be);
        assert_eq!(Fnv1a64.hash(b"a", 0), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(Murmur3_32.hash(b"", 0), 0);
        assert_eq!(Murmur3_32.hash(b"", 1), 0x514e_28b7);
        assert_eq!(Murmur3_32.hash(b"hello", 0), 0x248b_fa47);
        assert_eq!(Murmur3_32.hash(fox, 0), 0x2e4f_f723);
        assert_eq!(Xxh32.hash(b"", 0), 0x02cc_5d05);
        assert_eq!(Xxh64.hash(b"", 0), 0xef46_db37_51d8_e999);
        assert_eq!(Xxh3_64.hash(b"", 0), 0x2d06_8005_38d3_94c2);
        assert_eq!(Djb2.hash(b"a", 5381), 177_670);
        assert_eq!(Djb2a.hash(b"a", 5381), ((5381u32 * 33) ^ 97) as u64);
        assert_eq!(Sdbm.hash(b"a", 0), 97);
    }
}
