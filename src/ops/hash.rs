use crate::hashing::{self, Algorithm, CaseFold};
use crate::input;
use serde_json::{Value, json};

const MAX_INPUTS: usize = 1000;
const MAX_INPUT_BYTES: usize = 1024 * 1024;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "hash_compute" => hash_compute(args),
        _ => Err(format!("Unknown tool '{name}'.")),
    }
}

pub(crate) fn seed(args: &Value, algorithm: Algorithm) -> Result<u64, String> {
    if args.get("seed").is_none() {
        return Ok(algorithm.default_seed());
    }
    if !algorithm.seeded() {
        return Err("FNV hashes do not take a seed.".into());
    }
    let seed = crate::integer::unsigned(input::integer(args, "seed")?, algorithm.seed_bits())?;
    Ok(seed as u64)
}

fn optional_text<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| format!("'{key}' must be a string.")),
    }
}

fn hash_compute(args: &Value) -> Result<Value, String> {
    let algorithm = Algorithm::parse(input::string(args, "algorithm")?)?;
    let encoding = optional_text(args, "encoding")?.unwrap_or("utf8");
    let fold = CaseFold::parse(optional_text(args, "caseFold")?)?;
    let seed = seed(args, algorithm)?;
    let hash = |text: &str| -> Result<Value, String> {
        let mut data = hashing::encode_text(text, encoding)?;
        if data.len() > MAX_INPUT_BYTES {
            return Err("Each input may contain at most 1 MiB.".into());
        }
        if encoding != "hex" {
            fold.apply(&mut data);
        }
        let mut result = hashing::describe(algorithm.hash(&data, seed), algorithm.bits());
        result["input"] = json!(text);
        Ok(result)
    };
    let header = json!({"algorithm":input::string(args, "algorithm")?,"bits":algorithm.bits(),
        "seed":seed.to_string(),"encoding":encoding});
    match (optional_text(args, "data")?, args.get("inputs")) {
        (Some(text), None) => {
            let mut result = hash(text)?;
            for (key, value) in header.as_object().unwrap() {
                result[key] = value.clone();
            }
            Ok(result)
        }
        (None, Some(inputs)) => {
            let inputs = inputs
                .as_array()
                .ok_or("'inputs' must be an array of strings.")?;
            if inputs.is_empty() || inputs.len() > MAX_INPUTS {
                return Err("'inputs' must contain 1 to 1000 strings.".into());
            }
            let results = inputs
                .iter()
                .map(|value| {
                    hash(
                        value
                            .as_str()
                            .ok_or("'inputs' must contain only strings.")?,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut result = header;
            result["results"] = json!(results);
            Ok(result)
        }
        _ => Err("Supply exactly one of 'data' or 'inputs'.".into()),
    }
}
