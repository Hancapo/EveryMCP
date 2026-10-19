use super::{io_error, optional_str, optional_u64, required_str};
use goblin::pe::PE;
use goblin::pe::exception::{Register, UnwindHandler, UnwindOperation};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

const MAX_IMAGE_BYTES: u64 = 256 * 1024 * 1024;
const DEFAULT_SYMBOL_SERVER: &str = "https://msdl.microsoft.com/download/symbols";

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    let path = Path::new(required_str(args, "path")?);
    let bytes = read_image(path)?;
    let pe = PE::parse(&bytes).map_err(|e| io_error("Invalid PE image", e))?;
    match name {
        "pe_debug_info" => debug_info(path, &pe, args),
        "pe_resources" => resources(path, &bytes, &pe, args),
        "pe_runtime_functions" => runtime_functions(path, &pe, args),
        _ => unreachable!(),
    }
}

pub(super) fn read_image(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path).map_err(|e| io_error("Cannot inspect file", e))?;
    if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES {
        return Err("Input must be a file of at most 256 MiB.".into());
    }
    fs::read(path).map_err(|e| io_error("Cannot read file", e))
}

/// Maps an RVA to a file offset through the section table; header RVAs map directly.
pub(super) fn rva_to_offset(pe: &PE, rva: u32) -> Option<u32> {
    let headers = pe
        .header
        .optional_header
        .map(|h| h.windows_fields.size_of_headers)
        .unwrap_or(0);
    if rva < headers {
        return Some(rva);
    }
    pe.sections.iter().find_map(|s| {
        let span = s.virtual_size.max(s.size_of_raw_data);
        let delta = rva.checked_sub(s.virtual_address)?;
        (delta < span && delta < s.size_of_raw_data).then(|| s.pointer_to_raw_data + delta)
    })
}

fn section_name(pe: &PE, rva: u32) -> Option<String> {
    pe.sections
        .iter()
        .find(|s| {
            rva >= s.virtual_address
                && rva - s.virtual_address < s.virtual_size.max(s.size_of_raw_data)
        })
        .map(|s| s.name().unwrap_or("").to_owned())
}

fn debug_type_name(kind: u32) -> &'static str {
    match kind {
        1 => "coff",
        2 => "codeview",
        3 => "fpo",
        4 => "misc",
        5 => "exception",
        6 => "fixup",
        7 => "omap_to_src",
        8 => "omap_from_src",
        9 => "borland",
        11 => "clsid",
        12 => "vc_feature",
        13 => "pogo",
        14 => "iltcg",
        15 => "mpx",
        16 => "repro",
        17 => "embedded_portable_pdb",
        19 => "pdb_checksum",
        20 => "ex_dllcharacteristics",
        _ => "unknown",
    }
}

fn guid_string(raw: &[u8; 16]) -> (String, String) {
    let d1 = u32::from_le_bytes(raw[0..4].try_into().unwrap());
    let d2 = u16::from_le_bytes(raw[4..6].try_into().unwrap());
    let d3 = u16::from_le_bytes(raw[6..8].try_into().unwrap());
    let tail: String = raw[8..].iter().map(|b| format!("{b:02X}")).collect();
    (
        format!("{d1:08X}-{d2:04X}-{d3:04X}-{}-{}", &tail[..4], &tail[4..]),
        format!("{d1:08X}{d2:04X}{d3:04X}{tail}"),
    )
}

fn basename(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn debug_info(path: &Path, pe: &PE, args: &Value) -> Result<Value, String> {
    let server = optional_str(args, "symbolServer")?
        .unwrap_or(DEFAULT_SYMBOL_SERVER)
        .trim_end_matches('/');
    let timestamp = pe.header.coff_header.time_date_stamp;
    let size_of_image = pe
        .header
        .optional_header
        .map(|h| h.windows_fields.size_of_image)
        .unwrap_or(0);
    let image_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let image_key = format!("{timestamp:08X}{size_of_image:x}");
    let mut entries = Vec::new();
    let mut codeview = Value::Null;
    if let Some(debug) = &pe.debug_data {
        for entry in debug.entries().take(64) {
            let entry = entry.map_err(|e| io_error("Invalid debug directory", e))?;
            entries.push(json!({"type":entry.data_type,"typeName":debug_type_name(entry.data_type),
                "sizeBytes":entry.size_of_data,"rva":format!("0x{:x}",entry.address_of_raw_data),
                "fileOffset":entry.pointer_to_raw_data,"timeDateStamp":format!("0x{:08X}",entry.time_date_stamp)}));
        }
        if let Some(cv) = &debug.codeview_pdb70_debug_info {
            let pdb_path = String::from_utf8_lossy(cv.filename)
                .trim_end_matches('\0')
                .to_owned();
            let pdb_name = basename(&pdb_path).to_owned();
            let (guid, guid_key) = guid_string(&cv.signature);
            let key = format!("{guid_key}{:X}", cv.age);
            codeview = json!({"format":"RSDS","guid":guid,"age":cv.age,"pdbPath":pdb_path,"pdbName":pdb_name,
                "symbolServerKey":key,"symbolServerUrl":format!("{server}/{pdb_name}/{key}/{pdb_name}")});
        } else if let Some(cv) = &debug.codeview_pdb20_debug_info {
            let pdb_path = String::from_utf8_lossy(cv.filename)
                .trim_end_matches('\0')
                .to_owned();
            let pdb_name = basename(&pdb_path).to_owned();
            let key = format!("{:08X}{:X}", cv.signature, cv.age);
            codeview = json!({"format":"NB10","signature":format!("0x{:08X}",cv.signature),"age":cv.age,
                "pdbPath":pdb_path,"pdbName":pdb_name,"symbolServerKey":key,
                "symbolServerUrl":format!("{server}/{pdb_name}/{key}/{pdb_name}")});
        }
    }
    Ok(
        json!({"path":path.to_string_lossy(),"timeDateStamp":format!("0x{timestamp:08X}"),
        "sizeOfImage":format!("0x{size_of_image:x}"),"debugEntries":entries,"codeView":codeview,
        "imageSymbolServerKey":image_key,
        "imageSymbolServerUrl":format!("{server}/{image_name}/{image_key}/{image_name}")}),
    )
}

/// Formats a VS_FIXEDFILEINFO version pair. goblin's `file_version()` reads the date fields.
fn version_quad(ms: u32, ls: u32) -> String {
    format!("{}.{}.{}.{}", ms >> 16, ms & 0xffff, ls >> 16, ls & 0xffff)
}

fn resource_type_name(id: u32) -> Option<&'static str> {
    Some(match id {
        1 => "RT_CURSOR",
        2 => "RT_BITMAP",
        3 => "RT_ICON",
        4 => "RT_MENU",
        5 => "RT_DIALOG",
        6 => "RT_STRING",
        7 => "RT_FONTDIR",
        8 => "RT_FONT",
        9 => "RT_ACCELERATOR",
        10 => "RT_RCDATA",
        11 => "RT_MESSAGETABLE",
        12 => "RT_GROUP_CURSOR",
        14 => "RT_GROUP_ICON",
        16 => "RT_VERSION",
        17 => "RT_DLGINCLUDE",
        19 => "RT_PLUGPLAY",
        20 => "RT_VXD",
        21 => "RT_ANICURSOR",
        22 => "RT_ANIICON",
        23 => "RT_HTML",
        24 => "RT_MANIFEST",
        _ => return None,
    })
}

struct ResourceWalk<'a> {
    bytes: &'a [u8],
    root: usize,
    limit: usize,
    resources: Vec<Value>,
    total: usize,
}

impl ResourceWalk<'_> {
    fn u16_at(&self, offset: usize) -> Option<u16> {
        Some(u16::from_le_bytes(
            self.bytes.get(offset..offset + 2)?.try_into().ok()?,
        ))
    }

    fn u32_at(&self, offset: usize) -> Option<u32> {
        Some(u32::from_le_bytes(
            self.bytes.get(offset..offset + 4)?.try_into().ok()?,
        ))
    }

    fn name(&self, raw: u32) -> Value {
        if raw & 0x8000_0000 == 0 {
            return json!(raw);
        }
        let offset = self.root + (raw & 0x7fff_ffff) as usize;
        let Some(length) = self.u16_at(offset) else {
            return Value::Null;
        };
        let units: Vec<u16> = (0..length as usize)
            .filter_map(|i| self.u16_at(offset + 2 + i * 2))
            .collect();
        json!(String::from_utf16_lossy(&units))
    }

    fn walk(&mut self, pe: &PE, directory: usize, path: &mut Vec<Value>) -> Result<(), String> {
        if path.len() > 3 {
            return Err("Resource tree is deeper than three levels.".into());
        }
        let named = self
            .u16_at(directory + 12)
            .ok_or("Truncated resource directory.")?;
        let ids = self
            .u16_at(directory + 14)
            .ok_or("Truncated resource directory.")?;
        for index in 0..(named as usize + ids as usize) {
            let entry = directory + 16 + index * 8;
            let name = self.u32_at(entry).ok_or("Truncated resource entry.")?;
            let target = self.u32_at(entry + 4).ok_or("Truncated resource entry.")?;
            path.push(self.name(name));
            if target & 0x8000_0000 != 0 {
                self.walk(pe, self.root + (target & 0x7fff_ffff) as usize, path)?;
            } else {
                self.total += 1;
                if self.resources.len() < self.limit {
                    let data = self.root + target as usize;
                    let rva = self.u32_at(data).ok_or("Truncated resource data entry.")?;
                    let size = self
                        .u32_at(data + 4)
                        .ok_or("Truncated resource data entry.")?;
                    let code_page = self.u32_at(data + 8).unwrap_or(0);
                    let kind = &path[0];
                    let type_name = kind
                        .as_u64()
                        .and_then(|id| resource_type_name(id as u32))
                        .map(Value::from)
                        .unwrap_or_else(|| kind.clone());
                    self.resources.push(json!({"type":type_name,"name":path.get(1),"language":path.get(2),
                        "rva":format!("0x{rva:x}"),"fileOffset":rva_to_offset(pe, rva),"sizeBytes":size,
                        "codePage":code_page}));
                }
            }
            path.pop();
        }
        Ok(())
    }
}

fn resources(path: &Path, bytes: &[u8], pe: &PE, args: &Value) -> Result<Value, String> {
    let limit = optional_u64(args, "limit", 500, 100_000)? as usize;
    let table = pe
        .header
        .optional_header
        .and_then(|h| h.data_directories.get_resource_table().copied());
    let Some(table) = table.filter(|t| t.size > 0) else {
        return Ok(
            json!({"path":path.to_string_lossy(),"resourceCount":0,"resources":[],"versionInfo":null,"manifest":null,"truncated":false}),
        );
    };
    let root =
        rva_to_offset(pe, table.virtual_address).ok_or("Resource directory is not mapped.")?;
    let mut walk = ResourceWalk {
        bytes,
        root: root as usize,
        limit,
        resources: Vec::new(),
        total: 0,
    };
    walk.walk(pe, root as usize, &mut Vec::new())?;
    let mut version = Value::Null;
    let mut manifest = Value::Null;
    if let Some(data) = &pe.resource_data {
        if let Some(info) = &data.version_info {
            let fixed = info.fixed_info.as_ref().filter(|f| f.is_valid()).map(|f| {
                json!({"fileVersion":version_quad(f.file_version_ms, f.file_version_ls),"productVersion":version_quad(f.product_version_ms, f.product_version_ls),
                    "fileFlags":format!("0x{:x}",f.file_flags & f.file_flags_mask),"fileOs":format!("0x{:x}",f.file_os),
                    "fileType":f.file_type})
            });
            let s = &info.string_info;
            version = json!({"fixed":fixed,"strings":{"CompanyName":s.company_name(),"FileDescription":s.file_description(),
                "FileVersion":s.file_version(),"InternalName":s.internal_name(),"LegalCopyright":s.legal_copyright(),
                "OriginalFilename":s.original_filename(),"ProductName":s.product_name(),"ProductVersion":s.product_version(),
                "Comments":s.comments(),"PrivateBuild":s.private_build(),"SpecialBuild":s.special_build()}});
        }
        if let Some(data) = &data.manifest_data {
            let text = String::from_utf8_lossy(data.data);
            manifest = json!(
                text.trim_start_matches('\u{feff}')
                    .chars()
                    .take(65536)
                    .collect::<String>()
            );
        }
    }
    Ok(
        json!({"path":path.to_string_lossy(),"resourceDirectoryRva":format!("0x{:x}",table.virtual_address),
        "resourceCount":walk.total,"resources":walk.resources,"versionInfo":version,"manifest":manifest,
        "truncated":walk.total>walk.resources.len()}),
    )
}

fn register_name(register: Register) -> &'static str {
    register.name().trim_start_matches('$')
}

fn unwind_operation(operation: &UnwindOperation) -> Value {
    match operation {
        UnwindOperation::PushNonVolatile(r) => {
            json!({"op":"push_nonvol","register":register_name(*r)})
        }
        UnwindOperation::Alloc(size) => json!({"op":"alloc","sizeBytes":size}),
        UnwindOperation::SetFPRegister => json!({"op":"set_fp_register"}),
        UnwindOperation::SaveNonVolatile(r, o) => {
            json!({"op":"save_nonvol","register":register_name(*r),"offset":format!("{o:?}")})
        }
        UnwindOperation::SaveXMM(r, o) => {
            json!({"op":"save_xmm","register":register_name(*r),"offset":format!("{o:?}")})
        }
        UnwindOperation::SaveXMM128(r, o) => {
            json!({"op":"save_xmm128","register":register_name(*r),"offset":format!("{o:?}")})
        }
        UnwindOperation::PushMachineFrame(code) => {
            json!({"op":"push_machine_frame","errorCode":code})
        }
        other => json!({"op":format!("{other:?}")}),
    }
}

fn runtime_functions(path: &Path, pe: &PE, args: &Value) -> Result<Value, String> {
    let limit = optional_u64(args, "limit", 500, 100_000)? as usize;
    let offset = optional_u64(args, "offset", 0, u32::MAX as u64)? as usize;
    let unwind = super::optional_bool(args, "includeUnwind", false)?;
    let lookup = match optional_str(args, "rva")? {
        Some(raw) => Some(
            crate::input::parse_big_integer(raw, "rva")
                .ok()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("'rva' must be a 32-bit integer string.")?,
        ),
        None => None,
    };
    if pe.header.coff_header.machine != 0x8664 {
        return Err("pe_runtime_functions supports x64 (AMD64) images; .pdata layout differs on other machines.".into());
    }
    let Some(exceptions) = &pe.exception_data else {
        return Ok(
            json!({"path":path.to_string_lossy(),"functionCount":0,"functions":[],"nextOffset":null}),
        );
    };
    let describe = |function: goblin::pe::exception::RuntimeFunction| -> Value {
        let mut entry = json!({"begin":format!("0x{:x}",function.begin_address),"end":format!("0x{:x}",function.end_address),
            "sizeBytes":function.end_address.saturating_sub(function.begin_address),
            "unwindInfoRva":format!("0x{:x}",function.unwind_info_address & !1),
            "fileOffset":rva_to_offset(pe, function.begin_address),"section":section_name(pe, function.begin_address)});
        match exceptions.get_unwind_info(function, &pe.sections) {
            Ok(info) => {
                entry["chained"] = json!(info.chained_info.is_some());
                entry["primaryBegin"] = json!(
                    info.chained_info
                        .map(|c| format!("0x{:x}", c.begin_address))
                );
                if unwind {
                    let codes: Vec<_> = info
                        .unwind_codes()
                        .filter_map(Result::ok)
                        .map(|c| {
                            let mut op = unwind_operation(&c.operation);
                            op["codeOffset"] = json!(c.code_offset);
                            op
                        })
                        .collect();
                    let handler = info.handler.as_ref().map(|h| match h {
                        UnwindHandler::ExceptionHandler(rva, _) => {
                            json!({"kind":"exception","rva":format!("0x{rva:x}")})
                        }
                        UnwindHandler::TerminationHandler(rva, _) => {
                            json!({"kind":"termination","rva":format!("0x{rva:x}")})
                        }
                    });
                    entry["unwind"] = json!({"version":info.version,"prologSize":info.size_of_prolog,
                        "frameRegister":(info.frame_register.0 != 0).then(|| register_name(info.frame_register)),
                        "frameRegisterOffset":info.frame_register_offset,"codes":codes,"handler":handler});
                }
            }
            Err(error) => entry["unwindError"] = json!(error.to_string()),
        }
        entry
    };
    if let Some(rva) = lookup {
        let found = exceptions
            .find_function(rva)
            .map_err(|e| io_error("Invalid .pdata", e))?;
        return Ok(
            json!({"path":path.to_string_lossy(),"rva":format!("0x{rva:x}"),"function":found.map(describe)}),
        );
    }
    let total = exceptions.len();
    let functions: Vec<_> = exceptions
        .functions()
        .skip(offset)
        .take(limit)
        .map(|f| f.map(describe).map_err(|e| io_error("Invalid .pdata", e)))
        .collect::<Result<_, _>>()?;
    let next = (offset + functions.len() < total).then_some(offset + functions.len());
    Ok(
        json!({"path":path.to_string_lossy(),"functionCount":total,"functions":functions,"nextOffset":next}),
    )
}
