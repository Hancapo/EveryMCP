use super::{io_error, optional_u64, required_str};
use goblin::pe::PE;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

pub fn pe_inspect(args: &Value) -> Result<Value, String> {
    let path = Path::new(required_str(args, "path")?);
    let limit = optional_u64(args, "limit", 200, 2000)? as usize;
    if limit == 0 {
        return Err("'limit' must be positive.".into());
    }
    let metadata = fs::metadata(path).map_err(|e| io_error("Cannot inspect PE file", e))?;
    if !metadata.is_file() || metadata.len() > 256 * 1024 * 1024 {
        return Err("PE input must be a file of at most 256 MiB.".into());
    }
    let bytes = fs::read(path).map_err(|e| io_error("Cannot read PE file", e))?;
    let pe = PE::parse(&bytes).map_err(|e| io_error("Invalid PE image", e))?;
    let sections: Vec<_> = pe.sections.iter().take(limit).map(|s| json!({
        "name":s.name().unwrap_or(""),"virtualAddress":s.virtual_address,"virtualSize":s.virtual_size,
        "rawOffset":s.pointer_to_raw_data,"rawSize":s.size_of_raw_data,"characteristics":format!("0x{:08x}",s.characteristics)
    })).collect();
    let imports: Vec<_> = pe
        .imports
        .iter()
        .take(limit)
        .map(|i| json!({"dll":i.dll,"name":i.name,"ordinal":i.ordinal,"rva":i.rva}))
        .collect();
    let exports: Vec<_> = pe
        .exports
        .iter()
        .take(limit)
        .map(|e| json!({"name":e.name,"rva":e.rva,"fileOffset":e.offset}))
        .collect();
    let certificate = pe
        .header
        .optional_header
        .as_ref()
        .and_then(|h| h.data_directories.get_certificate_table());
    Ok(
        json!({"path":path.to_string_lossy(),"sizeBytes":bytes.len(),"machine":format!("0x{:04x}",pe.header.coff_header.machine),
        "architecture":match pe.header.coff_header.machine {0x8664=>"x86_64",0x14c=>"x86",0xaa64=>"aarch64",_=>"unknown"},
        "is64Bit":pe.is_64,"isDll":pe.is_lib,"imageBase":format!("0x{:x}",pe.image_base),
        "entryRva":format!("0x{:x}",pe.entry),"sectionCount":pe.sections.len(),"sections":sections,
        "importCount":pe.imports.len(),"imports":imports,"exportCount":pe.exports.len(),"exports":exports,
        "signaturePresent":certificate.is_some_and(|c| c.size > 0),
        "truncated":pe.sections.len()>limit || pe.imports.len()>limit || pe.exports.len()>limit}),
    )
}
