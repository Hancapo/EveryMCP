use serde_json::Value;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    #[cfg(windows)]
    {
        windows_impl::execute(name, args)
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        Err(format!("{name} is available only on Windows."))
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::super::file_extra::parse_pattern;
    use super::super::process_dump::windows_impl::{
        ProcessHandle, enumerate_modules, module_info, resolve_pid,
    };
    use super::super::{optional_bool, optional_str, optional_u64, required_str, string_array};
    use crate::bytes;
    use serde_json::{Value, json};
    use std::ffi::c_void;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows::Win32::System::Memory::{
        MEM_COMMIT, MEM_IMAGE, MEM_MAPPED, MEM_PRIVATE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE,
        PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_GUARD,
        PAGE_NOACCESS, VirtualQueryEx,
    };
    use windows::Win32::System::Threading::{
        IsWow64Process, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };
    use windows::core::BOOL;

    const MAX_READ_BYTES: u64 = 1024 * 1024;
    const PAGE: u64 = 4096;

    pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
        let process = Process::open(args)?;
        match name {
            "process_memory_read" => memory_read(&process, args),
            "pointer_chain_resolve" => pointer_chain(&process, args),
            "process_pattern_scan" => pattern_scan(&process, args),
            _ => unreachable!(),
        }
    }

    struct Module {
        name: String,
        path: String,
        base: u64,
        size: u64,
    }

    struct Region {
        base: u64,
        end: u64,
        readable: bool,
        executable: bool,
        info: MEMORY_BASIC_INFORMATION,
    }

    struct Process {
        pid: u32,
        handle: ProcessHandle,
        pointer_size: u64,
        modules: Vec<Module>,
    }

    impl Process {
        fn open(args: &Value) -> Result<Self, String> {
            let pid = resolve_pid(args)?;
            let handle = ProcessHandle(
                unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid) }
                    .map_err(|e| format!("Cannot open PID {pid} for query and VM read: {e}. Run with sufficient rights; SeDebugPrivilege may be needed."))?,
            );
            let mut wow64 = BOOL(0);
            unsafe { IsWow64Process(handle.0, &mut wow64) }
                .map_err(|e| format!("Cannot query process bitness: {e}"))?;
            let pointer_size = if wow64.as_bool() || cfg!(target_pointer_width = "32") {
                4
            } else {
                8
            };
            let mut modules = Vec::new();
            for (path, module) in enumerate_modules(handle.0)? {
                let Ok(info) = module_info(handle.0, module) else {
                    continue;
                };
                let name = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_owned();
                modules.push(Module {
                    name,
                    path,
                    base: info.lpBaseOfDll as u64,
                    size: info.SizeOfImage as u64,
                });
            }
            Ok(Self {
                pid,
                handle,
                pointer_size,
                modules,
            })
        }

        fn raw(&self) -> HANDLE {
            self.handle.0
        }

        fn module(&self, name: &str) -> Result<&Module, String> {
            let wanted = name.to_lowercase();
            let mut found = self.modules.iter().filter(|m| {
                let actual = m.name.to_lowercase();
                actual == wanted
                    || actual.strip_suffix(".exe") == Some(wanted.as_str())
                    || actual.strip_suffix(".dll") == Some(wanted.as_str())
            });
            let first = found
                .next()
                .ok_or_else(|| format!("Module '{name}' is not loaded in PID {}.", self.pid))?;
            if found.next().is_some() {
                return Err(format!("Module name '{name}' is ambiguous."));
            }
            Ok(first)
        }

        fn describe(&self, address: u64) -> Value {
            self.modules
                .iter()
                .find(|m| address >= m.base && address - m.base < m.size)
                .map_or(Value::Null, |m| {
                    json!(format!("{}+0x{:X}", m.name, address - m.base))
                })
        }

        /// Parses an absolute integer, a module name, or `module+offset` / `module-offset`.
        fn address(&self, raw: &str, key: &str) -> Result<u64, String> {
            let raw = raw.trim();
            let integer = |text: &str| -> Option<i128> {
                crate::input::parse_big_integer(text.trim(), key)
                    .ok()
                    .and_then(|v| num_traits::ToPrimitive::to_i128(&v))
            };
            let value = if let Some(value) = integer(raw) {
                value
            } else {
                let split = raw
                    .rfind(['+', '-'])
                    .filter(|&i| i > 0)
                    .and_then(|i| integer(&raw[i + 1..]).map(|offset| (i, offset)));
                match split {
                    Some((index, offset)) => {
                        let base = self.module(raw[..index].trim())?.base as i128;
                        if raw.as_bytes()[index] == b'+' {
                            base + offset
                        } else {
                            base - offset
                        }
                    }
                    None => self.module(raw)?.base as i128,
                }
            };
            u64::try_from(value)
                .ok()
                .filter(|v| self.pointer_size == 8 || *v <= u32::MAX as u64)
                .ok_or_else(|| format!("'{key}' is outside the target address space."))
        }

        fn region(&self, address: u64) -> Option<Region> {
            let mut info = MEMORY_BASIC_INFORMATION::default();
            let queried = unsafe {
                VirtualQueryEx(
                    self.raw(),
                    Some(address as usize as *const c_void),
                    &mut info,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if queried == 0 {
                return None;
            }
            let base = info.BaseAddress as u64;
            let end = base.checked_add(info.RegionSize as u64)?;
            if address < base || end <= address {
                return None;
            }
            let protect = info.Protect;
            let readable = info.State == MEM_COMMIT
                && !protect.contains(PAGE_NOACCESS)
                && !protect.contains(PAGE_GUARD)
                && protect.0 != 0;
            let executable = [
                PAGE_EXECUTE,
                PAGE_EXECUTE_READ,
                PAGE_EXECUTE_READWRITE,
                PAGE_EXECUTE_WRITECOPY,
            ]
            .iter()
            .any(|flag| protect.contains(*flag));
            Some(Region {
                base,
                end,
                readable,
                executable,
                info,
            })
        }

        /// Reads into `output`, zero-filling unreadable spans and returning them.
        fn read(&self, address: u64, output: &mut [u8]) -> Vec<(u64, u64)> {
            output.fill(0);
            let mut missing: Vec<(u64, u64)> = Vec::new();
            let mut note = |start: u64, len: u64| match missing.last_mut() {
                Some((s, l)) if *s + *l == start => *l += len,
                _ => missing.push((start, len)),
            };
            let mut position = 0usize;
            while position < output.len() {
                let current = address + position as u64;
                let span_to_page = (PAGE - current % PAGE) as usize;
                let Some(region) = self.region(current) else {
                    let len = span_to_page.min(output.len() - position);
                    note(current, len as u64);
                    position += len;
                    continue;
                };
                let span = ((region.end - current) as usize).min(output.len() - position);
                if !region.readable {
                    note(current, span as u64);
                    position += span;
                    continue;
                }
                let slice = &mut output[position..position + span];
                if !self.read_exact(current, slice) {
                    // Pages can change protection while the process runs; retry per page.
                    let mut offset = 0usize;
                    while offset < span {
                        let page_address = current + offset as u64;
                        let len = ((PAGE - page_address % PAGE) as usize).min(span - offset);
                        if !self.read_exact(page_address, &mut slice[offset..offset + len]) {
                            slice[offset..offset + len].fill(0);
                            note(page_address, len as u64);
                        }
                        offset += len;
                    }
                }
                position += span;
            }
            missing
        }

        fn read_exact(&self, address: u64, buffer: &mut [u8]) -> bool {
            let mut count = 0usize;
            let ok = unsafe {
                ReadProcessMemory(
                    self.raw(),
                    address as usize as *const c_void,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    Some(&mut count),
                )
            }
            .is_ok();
            ok && count == buffer.len()
        }

        fn pointer(&self, address: u64, size: u64) -> Result<u64, String> {
            let mut buffer = [0u8; 8];
            let size = size as usize;
            if !self.read_exact(address, &mut buffer[..size]) {
                return Err(format!("Cannot read pointer at 0x{address:X}."));
            }
            Ok(u64::from_le_bytes(buffer))
        }
    }

    fn hex(address: u64) -> String {
        format!("0x{address:X}")
    }

    fn missing_json(missing: &[(u64, u64)]) -> Vec<Value> {
        missing
            .iter()
            .map(|(start, len)| json!({"address":hex(*start),"sizeBytes":len}))
            .collect()
    }

    fn region_json(region: &Option<Region>) -> Value {
        region.as_ref().map_or(Value::Null, |r| {
            let kind = match r.info.Type {
                MEM_IMAGE => "MEM_IMAGE",
                MEM_MAPPED => "MEM_MAPPED",
                MEM_PRIVATE => "MEM_PRIVATE",
                _ => "NONE",
            };
            json!({"baseAddress":hex(r.base),"sizeBytes":r.end-r.base,"type":kind,
                "protect":format!("0x{:x}",r.info.Protect.0),"readable":r.readable,"executable":r.executable})
        })
    }

    /// Decodes typed values when 'type' is supplied.
    fn decode(args: &Value, data: &[u8]) -> Result<Value, String> {
        let Some(kind) = optional_str(args, "type")? else {
            return Ok(Value::Null);
        };
        let kind = bytes::ScalarType::parse(kind)?;
        let little = bytes::endian(optional_str(args, "endian")?.unwrap_or("little"))?;
        let width = kind.width() as u64;
        let stride = optional_u64(args, "stride", width, 4096)?;
        if stride < width {
            return Err("'stride' must be at least the type width.".into());
        }
        let available = (data.len() as u64).saturating_sub(width) / stride + 1;
        let count = optional_u64(args, "count", available.min(1024), 1024)?;
        (0..count)
            .map(|i| bytes::read_scalar(data, (i * stride) as usize, kind, little))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::from)
    }

    fn memory_read(process: &Process, args: &Value) -> Result<Value, String> {
        let address = process.address(required_str(args, "address")?, "address")?;
        let size = optional_u64(args, "size", 256, MAX_READ_BYTES)?;
        if size == 0 {
            return Err("'size' must be positive.".into());
        }
        address
            .checked_add(size)
            .ok_or("Read range overflows the address space.")?;
        let allow_partial = optional_bool(args, "allowPartial", false)?;
        let mut data = vec![0u8; size as usize];
        let missing = process.read(address, &mut data);
        if !missing.is_empty() && !allow_partial {
            return Err(format!(
                "Memory at 0x{:X} ({} bytes) is not readable; use allowPartial=true to zero-fill unreadable bytes.",
                missing[0].0, missing[0].1
            ));
        }
        Ok(
            json!({"pid":process.pid,"address":hex(address),"module":process.describe(address),
            "sizeBytes":size,"pointerSize":process.pointer_size,"region":region_json(&process.region(address)),
            "hex":bytes::format_hex(&data),"values":decode(args, &data)?,
            "unreadableBytes":missing.iter().map(|(_, l)| l).sum::<u64>(),"unreadableRanges":missing_json(&missing)}),
        )
    }

    fn pointer_chain(process: &Process, args: &Value) -> Result<Value, String> {
        let base = process.address(required_str(args, "base")?, "base")?;
        let raw_offsets = string_array(args, "offsets", false)?;
        if raw_offsets.len() > 64 {
            return Err("'offsets' may contain at most 64 entries.".into());
        }
        let offsets = raw_offsets
            .iter()
            .map(|raw| {
                let value = crate::input::parse_big_integer(raw, "offsets")?;
                num_traits::ToPrimitive::to_i64(&value)
                    .ok_or_else(|| "Offsets must fit in 64 bits.".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let pointer_size = optional_u64(args, "pointerSize", process.pointer_size, 8)?;
        if pointer_size != 4 && pointer_size != 8 {
            return Err("'pointerSize' must be 4 or 8.".into());
        }
        let wrap = |value: u64, offset: i64| -> u64 {
            let sum = value.wrapping_add(offset as u64);
            if pointer_size == 4 {
                sum & 0xffff_ffff
            } else {
                sum
            }
        };
        // Cheat Engine semantics: read [base], then for each offset but the last
        // read [previous + offset]; the last offset is added without a read.
        let mut hops = Vec::new();
        let mut failure = Value::Null;
        let mut current = base;
        let mut resolved = None;
        let steps = offsets.len().max(1);
        for index in 0..steps {
            let read_at = if index == 0 {
                base
            } else {
                wrap(current, offsets[index - 1])
            };
            match process.pointer(read_at, pointer_size) {
                Ok(value) => {
                    hops.push(json!({"index":index,"readAddress":hex(read_at),
                        "module":process.describe(read_at),"pointer":hex(value)}));
                    current = value;
                    if value == 0 {
                        failure = json!({"index":index,"reason":"null pointer"});
                        break;
                    }
                }
                Err(error) => {
                    failure = json!({"index":index,"reason":error});
                    break;
                }
            }
            if index + 1 == steps {
                resolved = Some(match offsets.last() {
                    Some(&last) => wrap(current, last),
                    None => current,
                });
            }
        }
        let value = match (resolved, optional_str(args, "type")?) {
            (Some(address), Some(kind)) => {
                let mut data = vec![0u8; bytes::ScalarType::parse(kind)?.width()];
                if !process.read(address, &mut data).is_empty() {
                    return Err(format!("Cannot read the final value at 0x{address:X}."));
                }
                decode(args, &data)?[0].clone()
            }
            _ => Value::Null,
        };
        Ok(
            json!({"pid":process.pid,"base":hex(base),"baseModule":process.describe(base),
            "pointerSize":pointer_size,"offsets":raw_offsets,"hops":hops,
            "address":resolved.map(hex),"module":resolved.map_or(Value::Null, |a| process.describe(a)),
            "value":value,"resolved":resolved.is_some(),"failure":failure}),
        )
    }

    fn pattern_scan(process: &Process, args: &Value) -> Result<Value, String> {
        let pattern = parse_pattern(required_str(args, "pattern")?)?;
        if pattern.iter().all(Option::is_none) {
            return Err("Pattern must include at least one fixed byte.".into());
        }
        let limit = optional_u64(args, "limit", 100, 1000)? as usize;
        if limit == 0 {
            return Err("'limit' must be positive.".into());
        }
        let executable_only = optional_bool(args, "executableOnly", false)?;
        let max_scan = optional_u64(args, "maxScanBytes", 2 << 30, 64 << 30)?;
        let top = if process.pointer_size == 4 {
            1u64 << 32
        } else {
            1u64 << 47
        };
        let (mut start, end, scope) = match (
            optional_str(args, "module")?,
            optional_str(args, "address")?,
        ) {
            (Some(name), None) => {
                let module = process.module(name)?;
                (
                    module.base,
                    module.base + module.size,
                    json!({"module":module.name,"path":module.path}),
                )
            }
            (None, Some(raw)) => {
                let start = process.address(raw, "address")?;
                let size = super::super::required_u64(args, "size")?;
                let end = start.checked_add(size).ok_or("Scan range overflows.")?;
                (start, end, json!({"address":hex(start),"sizeBytes":size}))
            }
            (None, None) => (0, top, json!("process")),
            _ => return Err("Supply 'module' or 'address' with 'size', not both.".into()),
        };
        let range_start = start;
        if let Some(raw) = optional_str(args, "startAddress")? {
            let resume = process.address(raw, "startAddress")?;
            if resume < start || resume > end {
                return Err("'startAddress' is outside the scan range.".into());
            }
            start = resume;
        }
        let mut matches = Vec::new();
        let mut scanned = 0u64;
        let mut regions = 0u64;
        let mut skipped = 0u64;
        let mut next = None;
        let mut address = start;
        let chunk = 1usize << 20;
        let overlap = pattern.len() - 1;
        // Carries the tail of the previous chunk so matches can span chunks and
        // adjacent readable regions.
        let mut buffer: Vec<u8> = Vec::with_capacity(chunk + overlap);
        'regions: while address < end {
            let Some(region) = process.region(address) else {
                break;
            };
            let region_end = region.end.min(end);
            if !region.readable || (executable_only && !region.executable) {
                if region.readable {
                    skipped += region_end - address;
                }
                buffer.clear();
                address = region_end;
                continue;
            }
            regions += 1;
            let mut position = address;
            while position < region_end {
                if scanned >= max_scan {
                    next = Some(position - buffer.len() as u64);
                    break 'regions;
                }
                let len = ((region_end - position) as usize).min(chunk);
                let carried = buffer.len();
                buffer.resize(carried + len, 0);
                let missing = process.read(position, &mut buffer[carried..]);
                skipped += missing.iter().map(|(_, l)| l).sum::<u64>();
                scanned += len as u64;
                let base = position - carried as u64;
                if buffer.len() >= pattern.len() {
                    for i in 0..=buffer.len() - pattern.len() {
                        let hit = pattern
                            .iter()
                            .enumerate()
                            .all(|(j, byte)| byte.is_none_or(|v| buffer[i + j] == v));
                        if hit {
                            let found = base + i as u64;
                            if matches.len() == limit {
                                next = Some(found);
                                break 'regions;
                            }
                            matches.push(
                                json!({"address":hex(found),"module":process.describe(found)}),
                            );
                        }
                    }
                }
                let keep = overlap.min(buffer.len());
                buffer.drain(..buffer.len() - keep);
                position += len as u64;
            }
            address = region_end;
        }
        Ok(
            json!({"pid":process.pid,"scope":scope,"rangeStart":hex(range_start),"rangeEnd":hex(end),
            "startAddress":hex(start),"patternLength":pattern.len(),"matches":matches,
            "limitReached":next.is_some() && matches.len() == limit,"nextAddress":next.map(hex),
            "bytesScanned":scanned,"regionsScanned":regions,"skippedBytes":skipped,
            "maxScanBytesReached":next.is_some() && scanned >= max_scan}),
        )
    }
}
