use super::{optional_bool, optional_str, optional_u64, required_str};
use serde_json::Value;

const MAX_HEADER_BYTES: usize = 4 * 1024 * 1024;
const MAX_SECTIONS: usize = 256;

fn word(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let data = bytes
        .get(offset..offset + 2)
        .ok_or("Truncated PE header.")?;
    Ok(u16::from_le_bytes([data[0], data[1]]))
}

fn dword(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let data = bytes
        .get(offset..offset + 4)
        .ok_or("Truncated PE header.")?;
    Ok(u32::from_le_bytes(data.try_into().unwrap()))
}

#[cfg(test)]
fn put_word(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_dword(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn align(value: u32, alignment: u32) -> Result<u32, String> {
    crate::integer::align_up(value as u128, alignment as u128)
        .and_then(|result| u32::try_from(result).ok())
        .ok_or("PE alignment overflow.".into())
}

fn required_header_bytes(bytes: &[u8], image_size: u32) -> Result<usize, String> {
    if bytes.len() < 64 || !bytes.starts_with(b"MZ") {
        return Err("The main module has no readable DOS/PE header.".into());
    }
    let pe = dword(bytes, 0x3c)? as usize;
    if pe > MAX_HEADER_BYTES - 24 {
        return Err("PE header offset is outside the supported header range.".into());
    }
    let coff_end = pe + 24;
    if bytes.len() < coff_end {
        return Ok(coff_end);
    }
    if &bytes[pe..pe + 4] != b"PE\0\0" {
        return Err("Invalid PE signature in process memory.".into());
    }
    let sections = word(bytes, pe + 6)? as usize;
    if sections == 0 || sections > MAX_SECTIONS {
        return Err("Unsupported PE section count.".into());
    }
    let opt_size = word(bytes, pe + 20)? as usize;
    let opt = coff_end;
    let table_end = opt
        .checked_add(opt_size)
        .and_then(|end| end.checked_add(sections * 40))
        .ok_or("PE header size overflow.")?;
    if table_end > MAX_HEADER_BYTES || table_end > image_size as usize {
        return Err("PE section table exceeds the image/header limit.".into());
    }
    if bytes.len() < opt + 64 {
        return Ok(table_end.max(opt + 64));
    }
    let size_of_headers = dword(bytes, opt + 60)? as usize;
    let needed = table_end.max(size_of_headers);
    if needed > MAX_HEADER_BYTES || needed > image_size as usize {
        return Err("SizeOfHeaders exceeds the image/header limit.".into());
    }
    Ok(needed)
}

#[derive(Clone, Debug)]
struct SectionPlan {
    name: String,
    header_offset: usize,
    rva: u32,
    virtual_size: u32,
    copy_size: u32,
    raw_offset: u32,
    raw_size: u32,
}

#[derive(Debug)]
struct PePlan {
    pe_offset: usize,
    optional_offset: usize,
    is_64: bool,
    section_alignment: u32,
    image_size: u32,
    header_size: u32,
    output_size: u32,
    preferred_image_base: u64,
    sections: Vec<SectionPlan>,
}

impl PePlan {
    fn parse(headers: &[u8], image_size: u32) -> Result<Self, String> {
        let needed = required_header_bytes(headers, image_size)?;
        if headers.len() < needed {
            return Err("Incomplete PE headers.".into());
        }
        let pe = dword(headers, 0x3c)? as usize;
        let section_count = word(headers, pe + 6)? as usize;
        let optional_size = word(headers, pe + 20)? as usize;
        let opt = pe + 24;
        let magic = word(headers, opt)?;
        let is_64 = match magic {
            0x10b => false,
            0x20b => true,
            _ => return Err("Unsupported PE optional-header format.".into()),
        };
        if optional_size < if is_64 { 112 } else { 96 } {
            return Err("PE optional header is too short.".into());
        }
        let file_alignment = dword(headers, opt + 36)?;
        let section_alignment = dword(headers, opt + 32)?;
        if !(512..=65536).contains(&file_alignment)
            || !file_alignment.is_power_of_two()
            || section_alignment < file_alignment
            || !section_alignment.is_power_of_two()
            || (section_alignment < 4096 && section_alignment != file_alignment)
        {
            return Err("Invalid PE file or section alignment.".into());
        }
        let original_image_size = dword(headers, opt + 56)?;
        if original_image_size == 0 || original_image_size > image_size {
            return Err("PE SizeOfImage exceeds the enumerated main module.".into());
        }
        let table = opt + optional_size;
        let table_end = table + section_count * 40;
        let header_size = align(needed as u32, file_alignment)?;
        if header_size > image_size || header_size as usize > MAX_HEADER_BYTES {
            return Err("Aligned PE headers exceed the image/header limit.".into());
        }
        let preferred_image_base = if is_64 {
            let low = dword(headers, opt + 24)? as u64;
            let high = dword(headers, opt + 28)? as u64;
            low | high << 32
        } else {
            dword(headers, opt + 28)? as u64
        };
        let mut sections = Vec::with_capacity(section_count);
        let mut previous_rva = 0u32;
        for index in 0..section_count {
            let offset = table + index * 40;
            let name_bytes = &headers[offset..offset + 8];
            let name_len = name_bytes.iter().position(|&v| v == 0).unwrap_or(8);
            let name = String::from_utf8_lossy(&name_bytes[..name_len]).to_string();
            let virtual_size = dword(headers, offset + 8)?;
            let rva = dword(headers, offset + 12)?;
            let original_raw_size = dword(headers, offset + 16)?;
            if rva < header_size
                || rva >= image_size
                || rva % section_alignment != 0
                || (index > 0 && rva <= previous_rva)
            {
                return Err(format!(
                    "Invalid or overlapping PE section RVA at index {index}."
                ));
            }
            previous_rva = rva;
            sections.push(SectionPlan {
                name,
                header_offset: offset,
                rva,
                virtual_size,
                copy_size: virtual_size.max(original_raw_size),
                raw_offset: 0,
                raw_size: 0,
            });
        }
        for index in 0..section_count {
            let end = sections.get(index + 1).map_or(image_size, |next| next.rva);
            let span = end - sections[index].rva;
            sections[index].copy_size = sections[index].copy_size.min(span);
            sections[index].virtual_size = sections[index].virtual_size.min(span);
        }
        let low_alignment = section_alignment < 4096;
        let mut cursor = header_size;
        for section in &mut sections {
            let raw_size = align(section.copy_size, file_alignment)?;
            let raw_offset = if raw_size == 0 {
                0
            } else if low_alignment {
                section.rva
            } else {
                cursor
            };
            let end = raw_offset
                .checked_add(raw_size)
                .ok_or("PE output size overflow.")?;
            if low_alignment && end > image_size {
                return Err("Low-alignment PE section exceeds the image.".into());
            }
            section.raw_offset = raw_offset;
            section.raw_size = raw_size;
            cursor = cursor.max(end);
        }
        if low_alignment
            && sections
                .windows(2)
                .any(|pair| pair[0].raw_offset + pair[0].raw_size > pair[1].raw_offset)
        {
            return Err("Low-alignment PE sections overlap in file layout.".into());
        }
        if table_end > header_size as usize {
            return Err("PE section table is not contained in headers.".into());
        }
        Ok(Self {
            pe_offset: pe,
            optional_offset: opt,
            is_64,
            section_alignment,
            image_size,
            header_size,
            output_size: cursor,
            preferred_image_base,
            sections,
        })
    }

    fn rewritten_headers(&self, original: &[u8], loaded_base: usize) -> Result<Vec<u8>, String> {
        if original.len() < self.header_size as usize {
            return Err("Incomplete PE header bytes.".into());
        }
        let mut headers = original[..self.header_size as usize].to_vec();
        let opt = self.optional_offset;
        if self.is_64 {
            headers[opt + 24..opt + 32].copy_from_slice(&(loaded_base as u64).to_le_bytes());
        } else {
            let base = u32::try_from(loaded_base)
                .map_err(|_| "32-bit PE has an invalid loaded base.".to_owned())?;
            put_dword(&mut headers, opt + 28, base);
        }
        put_dword(
            &mut headers,
            opt + 56,
            align(self.image_size, self.section_alignment)?,
        );
        put_dword(&mut headers, opt + 60, self.header_size);
        put_dword(&mut headers, opt + 64, 0); // The original on-disk checksum no longer applies.
        put_dword(&mut headers, self.pe_offset + 12, 0); // COFF symbol table is not mapped.
        put_dword(&mut headers, self.pe_offset + 16, 0);
        let directory_count = dword(&headers, opt + if self.is_64 { 108 } else { 92 })?;
        let directory_start = opt + if self.is_64 { 112 } else { 96 };
        for index in [4usize, 6usize] {
            // Certificates use file offsets; debug entries retain stale file pointers.
            let pos = directory_start + index * 8;
            if directory_count as usize > index
                && pos + 8 <= opt + word(&headers, self.pe_offset + 20)? as usize
            {
                headers[pos..pos + 8].fill(0);
            }
        }
        for section in &self.sections {
            let offset = section.header_offset;
            put_dword(&mut headers, offset + 8, section.virtual_size);
            put_dword(&mut headers, offset + 16, section.raw_size);
            put_dword(&mut headers, offset + 20, section.raw_offset);
            headers[offset + 24..offset + 36].fill(0); // COFF relocations/line numbers are not mapped.
        }
        Ok(headers)
    }
}

pub fn dump(args: &Value) -> Result<Value, String> {
    #[cfg(windows)]
    {
        windows_impl::dump(args)
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        Err("process_module_dump is available only on Windows.".into())
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use crate::host::io_error;
    use serde_json::json;
    use std::ffi::c_void;
    use std::io::{Seek, SeekFrom, Write};
    use std::path::Path;
    use sysinfo::System;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HMODULE};
    use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows::Win32::System::Memory::{
        MEM_COMMIT, MEM_IMAGE, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQueryEx,
    };
    use windows::Win32::System::ProcessStatus::{
        K32EnumProcessModulesEx, K32GetModuleFileNameExW, K32GetModuleInformation,
        LIST_MODULES_ALL, MODULEINFO,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ, QueryFullProcessImageNameW,
    };

    struct ProcessHandle(HANDLE);
    impl Drop for ProcessHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    fn matches_name(actual: &str, requested: &str) -> bool {
        let actual = actual.to_lowercase();
        let requested = requested.to_lowercase();
        actual == requested || actual.strip_suffix(".exe") == Some(requested.as_str())
    }

    fn resolve_pid(args: &Value) -> Result<u32, String> {
        let pid = optional_u64(args, "pid", 0, u32::MAX as u64)? as u32;
        let name = optional_str(args, "name")?;
        if pid == 0 && name.is_none() {
            return Err("Supply 'pid' or 'name'.".into());
        }
        let system = System::new_all();
        if pid != 0 {
            if let Some(name) = name {
                let process = system
                    .processes()
                    .iter()
                    .find(|(key, _)| key.as_u32() == pid)
                    .ok_or("PID is not running.")?
                    .1;
                if !matches_name(&process.name().to_string_lossy(), name) {
                    return Err("PID and process name refer to different processes.".into());
                }
            }
            return Ok(pid);
        }
        let name = name.unwrap();
        let candidates: Vec<u32> = system
            .processes()
            .iter()
            .filter(|(_, process)| matches_name(&process.name().to_string_lossy(), name))
            .map(|(pid, _)| pid.as_u32())
            .collect();
        match candidates.as_slice() {
            [pid] => Ok(*pid),
            [] => Err(format!("No running process named '{name}' was found.")),
            _ => Err(format!(
                "Process name is ambiguous; matching PIDs: {:?}.",
                candidates
            )),
        }
    }

    fn process_image_path(handle: HANDLE) -> Result<String, String> {
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                handle,
                Default::default(),
                windows::core::PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
        }
        .map_err(|e| format!("Cannot query executable path: {e}"))?;
        Ok(String::from_utf16_lossy(&buffer[..length as usize]))
    }

    fn main_module(handle: HANDLE, image_path: &str) -> Result<MODULEINFO, String> {
        let normalize = |s: &str| {
            s.trim_start_matches("\\\\?\\")
                .replace('/', "\\")
                .to_lowercase()
        };
        let expected = normalize(image_path);
        let mut modules = vec![HMODULE(std::ptr::null_mut()); 256];
        let count = loop {
            let mut needed = 0u32;
            let capacity = (modules.len() * std::mem::size_of::<HMODULE>()) as u32;
            if !unsafe {
                K32EnumProcessModulesEx(
                    handle,
                    modules.as_mut_ptr(),
                    capacity,
                    &mut needed,
                    LIST_MODULES_ALL.0,
                )
            }
            .as_bool()
            {
                return Err(format!(
                    "Cannot enumerate modules: {}",
                    windows::core::Error::from_thread()
                ));
            }
            if needed <= capacity {
                break needed as usize / std::mem::size_of::<HMODULE>();
            }
            let required = needed as usize / std::mem::size_of::<HMODULE>() + 16;
            if required > 32768 {
                return Err("Process has too many modules.".into());
            }
            modules.resize(required, HMODULE(std::ptr::null_mut()));
        };
        for module in modules.into_iter().take(count) {
            let mut name = vec![0u16; 32768];
            let len =
                unsafe { K32GetModuleFileNameExW(Some(handle), Some(module), &mut name) } as usize;
            if len == 0 || len >= name.len() {
                continue;
            }
            if normalize(&String::from_utf16_lossy(&name[..len])) != expected {
                continue;
            }
            let mut info = MODULEINFO::default();
            if !unsafe {
                K32GetModuleInformation(
                    handle,
                    module,
                    &mut info,
                    std::mem::size_of::<MODULEINFO>() as u32,
                )
            }
            .as_bool()
            {
                return Err(format!(
                    "Cannot inspect main module: {}",
                    windows::core::Error::from_thread()
                ));
            }
            return Ok(info);
        }
        Err("The executable path did not match any enumerated module.".into())
    }

    struct ImageReader {
        handle: HANDLE,
        base: usize,
        size: usize,
        allow_partial: bool,
        read_bytes: u64,
        missing_bytes: u64,
    }
    impl ImageReader {
        fn new(handle: HANDLE, base: usize, size: usize, allow_partial: bool) -> Self {
            Self {
                handle,
                base,
                size,
                allow_partial,
                read_bytes: 0,
                missing_bytes: 0,
            }
        }
        fn read(&mut self, rva: usize, output: &mut [u8]) -> Result<(), String> {
            if rva
                .checked_add(output.len())
                .is_none_or(|end| end > self.size)
            {
                return Err("Read would leave the main EXE image.".into());
            }
            output.fill(0);
            let mut position = 0usize;
            while position < output.len() {
                let address = self
                    .base
                    .checked_add(rva + position)
                    .ok_or("Process address overflow.")?;
                let mut region = MEMORY_BASIC_INFORMATION::default();
                if unsafe {
                    VirtualQueryEx(
                        self.handle,
                        Some(address as *const c_void),
                        &mut region,
                        std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                    )
                } == 0
                {
                    return Err(format!("VirtualQueryEx failed at 0x{address:x}."));
                }
                let region_start = region.BaseAddress as usize;
                let region_end = region_start
                    .checked_add(region.RegionSize)
                    .ok_or("Memory region overflow.")?;
                if address < region_start || region_end <= address {
                    return Err("VirtualQueryEx returned an invalid region.".into());
                }
                let span = (region_end - address).min(output.len() - position);
                let readable = region.State == MEM_COMMIT
                    && region.Type == MEM_IMAGE
                    && region.AllocationBase as usize == self.base
                    && !region.Protect.contains(PAGE_NOACCESS)
                    && !region.Protect.contains(PAGE_GUARD);
                if !readable {
                    if !self.allow_partial {
                        return Err(format!(
                            "Main EXE page at 0x{address:x} is not readable as MEM_IMAGE; use allowPartial=true to zero-fill it."
                        ));
                    }
                    self.missing_bytes += span as u64;
                    position += span;
                    continue;
                }
                let mut offset = 0usize;
                while offset < span {
                    let len = (span - offset).min(65536);
                    let remote = address + offset;
                    let slice = &mut output[position + offset..position + offset + len];
                    let mut count = 0usize;
                    let result = unsafe {
                        ReadProcessMemory(
                            self.handle,
                            remote as *const c_void,
                            slice.as_mut_ptr().cast(),
                            len,
                            Some(&mut count),
                        )
                    };
                    if result.is_ok() && count == len {
                        self.read_bytes += len as u64;
                    } else {
                        // A region can change while the process runs. Retry page-sized reads.
                        for (page, bytes) in slice.chunks_mut(4096).enumerate() {
                            let mut got = 0usize;
                            let page_address = remote + page * 4096;
                            let ok = unsafe {
                                ReadProcessMemory(
                                    self.handle,
                                    page_address as *const c_void,
                                    bytes.as_mut_ptr().cast(),
                                    bytes.len(),
                                    Some(&mut got),
                                )
                            }
                            .is_ok();
                            if ok && got == bytes.len() {
                                self.read_bytes += got as u64;
                            } else if self.allow_partial {
                                bytes.fill(0);
                                self.missing_bytes += bytes.len() as u64;
                            } else {
                                return Err(format!(
                                    "ReadProcessMemory failed at 0x{page_address:x}; use allowPartial=true to zero-fill unreadable pages."
                                ));
                            }
                        }
                    }
                    offset += len;
                }
                position += span;
            }
            Ok(())
        }
    }

    fn guard_output(output: &Path, executable: &Path, overwrite: bool) -> Result<(), String> {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = parent
            .canonicalize()
            .map_err(|e| io_error("Cannot resolve output directory", e))?;
        let filename = output.file_name().ok_or("outputPath must name a file.")?;
        let target = if output.exists() {
            output
                .canonicalize()
                .map_err(|e| io_error("Cannot resolve output", e))?
        } else {
            parent.join(filename)
        };
        let original = executable
            .canonicalize()
            .unwrap_or_else(|_| executable.to_path_buf());
        if target
            .to_string_lossy()
            .eq_ignore_ascii_case(&original.to_string_lossy())
        {
            return Err("Output must not overwrite the running executable.".into());
        }
        if output.exists() && !overwrite {
            return Err("Output already exists.".into());
        }
        Ok(())
    }

    pub fn dump(args: &Value) -> Result<Value, String> {
        let pid = resolve_pid(args)?;
        let output = Path::new(required_str(args, "outputPath")?);
        let overwrite = optional_bool(args, "overwrite", false)?;
        let allow_partial = optional_bool(args, "allowPartial", false)?;
        let max_image = optional_u64(
            args,
            "maxImageBytes",
            512 * 1024 * 1024,
            2 * 1024 * 1024 * 1024,
        )?;
        if max_image == 0 {
            return Err("maxImageBytes must be positive.".into());
        }
        let handle = ProcessHandle(unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid) }
            .map_err(|e| format!("Cannot open PID {pid} for query and VM read: {e}. Run with sufficient rights; SeDebugPrivilege may be needed."))?);
        let image_path = process_image_path(handle.0)?;
        if let Some(name) = optional_str(args, "name")? {
            let basename = Path::new(&image_path)
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or("Cannot identify the opened process executable name.")?;
            if !matches_name(basename, name) {
                return Err(
                    "Opened process executable no longer matches the requested name.".into(),
                );
            }
        }
        let module = main_module(handle.0, &image_path)?;
        let base = module.lpBaseOfDll as usize;
        let image_size = module.SizeOfImage as usize;
        if base == 0 || image_size < 4096 || image_size as u64 > max_image {
            return Err("Main module SizeOfImage is invalid or exceeds maxImageBytes.".into());
        }
        guard_output(output, Path::new(&image_path), overwrite)?;
        let mut reader = ImageReader::new(handle.0, base, image_size, allow_partial);
        let mut headers = vec![0u8; 4096];
        reader.read(0, &mut headers)?;
        loop {
            let needed = required_header_bytes(&headers, module.SizeOfImage)?;
            if needed <= headers.len() {
                break;
            }
            let previous = headers.len();
            headers.resize(needed, 0);
            reader.read(previous, &mut headers[previous..])?;
        }
        let plan = PePlan::parse(&headers, module.SizeOfImage)?;
        if headers.len() < plan.header_size as usize {
            let previous = headers.len();
            headers.resize(plan.header_size as usize, 0);
            reader.read(previous, &mut headers[previous..])?;
        }
        let output_headers = plan.rewritten_headers(&headers, base)?;
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| io_error("Cannot create dump temporary file", e))?;
        temp.as_file_mut()
            .set_len(plan.output_size as u64)
            .map_err(|e| io_error("Cannot size PE dump", e))?;
        temp.write_all(&output_headers)
            .map_err(|e| io_error("Cannot write PE headers", e))?;
        let mut buffer = vec![0u8; 1024 * 1024];
        for section in &plan.sections {
            let mut copied = 0u32;
            while copied < section.copy_size {
                let len = (section.copy_size - copied).min(buffer.len() as u32) as usize;
                reader.read((section.rva + copied) as usize, &mut buffer[..len])?;
                temp.as_file_mut()
                    .seek(SeekFrom::Start((section.raw_offset + copied) as u64))
                    .map_err(|e| io_error("Cannot seek PE dump", e))?;
                temp.write_all(&buffer[..len])
                    .map_err(|e| io_error("Cannot write PE section", e))?;
                copied += len as u32;
            }
        }
        temp.flush()
            .map_err(|e| io_error("Cannot flush PE dump", e))?;
        let persisted = if overwrite {
            temp.persist(output)
        } else {
            temp.persist_noclobber(output)
        };
        persisted.map_err(|e| io_error("Cannot persist PE dump", e.error))?;
        let (sha256, bytes) = super::super::files::hash_file(output, "sha256")?;
        let sections: Vec<_> = plan
            .sections
            .iter()
            .map(|s| {
                json!({"name":s.name,"rva":format!("0x{:x}",s.rva),
            "virtualSize":s.virtual_size,"fileOffset":s.raw_offset,"rawSize":s.raw_size})
            })
            .collect();
        Ok(
            json!({"pid":pid,"sourceImage":image_path,"imageBase":format!("0x{base:x}"),
            "preferredImageBase":format!("0x{:x}",plan.preferred_image_base),"sizeOfImage":plan.image_size,
            "outputPath":output.to_string_lossy(),"outputBytes":bytes,"sha256":sha256,
            "sectionCount":sections.len(),"sections":sections,"bytesRead":reader.read_bytes,
            "unreadableBytes":reader.missing_bytes,"partial":reader.missing_bytes>0,
            "note":"Analysis snapshot of the loaded main EXE; imports, original entry point, debug data and runnable state are not reconstructed."}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repacks_memory_sections_into_file_offsets() {
        let mut header = vec![0u8; 0x400];
        header[..2].copy_from_slice(b"MZ");
        put_dword(&mut header, 0x3c, 0x80);
        header[0x80..0x84].copy_from_slice(b"PE\0\0");
        put_word(&mut header, 0x86, 2);
        put_word(&mut header, 0x94, 0xf0);
        put_word(&mut header, 0x98, 0x20b);
        header[0x98 + 24..0x98 + 32].copy_from_slice(&0x140000000u64.to_le_bytes());
        put_dword(&mut header, 0x98 + 32, 0x1000);
        put_dword(&mut header, 0x98 + 36, 0x200);
        put_dword(&mut header, 0x98 + 56, 0x4000);
        put_dword(&mut header, 0x98 + 60, 0x400);
        let table = 0x98 + 0xf0;
        header[table..table + 5].copy_from_slice(b".text");
        put_dword(&mut header, table + 8, 0x900);
        put_dword(&mut header, table + 12, 0x1000);
        put_dword(&mut header, table + 16, 0x600);
        header[table + 40..table + 45].copy_from_slice(b".data");
        put_dword(&mut header, table + 48, 0x700);
        put_dword(&mut header, table + 52, 0x2000);
        put_dword(&mut header, table + 56, 0x200);
        let plan = PePlan::parse(&header, 0x4000).unwrap();
        assert_eq!(plan.sections[0].raw_offset, 0x400);
        assert_eq!(plan.sections[0].raw_size, 0xa00);
        assert_eq!(plan.sections[1].raw_offset, 0xe00);
        assert_eq!(plan.sections[1].raw_size, 0x800);
        assert_eq!(plan.output_size, 0x1600);
        let rebuilt = plan.rewritten_headers(&header, 0x180000000).unwrap();
        assert_eq!(dword(&rebuilt, table + 20).unwrap(), 0x400);
        assert_eq!(dword(&rebuilt, table + 60).unwrap(), 0xe00);
        assert_eq!(dword(&rebuilt, 0x98 + 24).unwrap(), 0x80000000);
        assert_eq!(dword(&rebuilt, 0x98 + 28).unwrap(), 1);
    }
}
