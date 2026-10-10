use super::{data, files, io_error, optional_str, required_str};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn patch(args: &Value) -> Result<Value, String> {
    let path = Path::new(required_str(args, "path")?);
    let source = data::read_text(path)?;
    let digest = files::digest_hex(&Sha256::digest(source.as_bytes()));
    if let Some(expected) = optional_str(args, "expectedSha256")?
        && !expected.eq_ignore_ascii_case(&digest)
    {
        return Err("File SHA-256 does not match expectedSha256.".into());
    }
    let operations = args
        .get("operations")
        .and_then(Value::as_array)
        .ok_or("'operations' must be an array.")?;
    if operations.is_empty() || operations.len() > 1000 {
        return Err("'operations' must contain 1-1000 operations.".into());
    }
    let patch: json_patch::Patch =
        serde_json::from_value(json!(operations)).map_err(|e| io_error("Invalid JSON patch", e))?;
    let mut document: Value = crate::json_value::parse(&source)?;
    let mut bytes = Vec::new();
    for (index, operation) in patch.0.iter().enumerate() {
        // The library compares number representations; RFC 6902 requires numeric equality.
        if let json_patch::PatchOperation::Test(test) = operation {
            if !document
                .pointer(test.path.as_str())
                .is_some_and(|value| crate::json_value::equivalent(value, &test.value))
            {
                return Err(format!(
                    "JSON patch test failed at operation {index}, path '{}'.",
                    test.path
                ));
            }
        } else {
            json_patch::patch(&mut document, std::slice::from_ref(operation))
                .map_err(|e| io_error("JSON patch failed", e))?;
        }
        bytes = serde_json::to_vec_pretty(&document)
            .map_err(|e| io_error("Cannot encode patched document", e))?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("Patched document exceeds 8 MiB.".into());
        }
    }
    if data::read_text(path)? != source {
        return Err("Source changed while the patch was being prepared.".into());
    }
    files::persist_bytes(path, &bytes, true)?;
    Ok(
        json!({"path":path.to_string_lossy(),"operationsApplied":operations.len(),"bytesWritten":bytes.len(),
        "oldSha256":digest,"newSha256":files::digest_hex(&Sha256::digest(&bytes))}),
    )
}
