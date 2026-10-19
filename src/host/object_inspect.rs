use super::pe_extra::read_image;
use super::{io_error, optional_u64, required_str};
use goblin::elf::{self, Elf};
use goblin::mach::{self, Mach, MachO, SingleArch};
use serde_json::{Value, json};
use std::path::Path;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    let path = Path::new(required_str(args, "path")?);
    let limit = optional_u64(args, "limit", 200, 100_000)? as usize;
    let bytes = read_image(path)?;
    match name {
        "elf_inspect" => elf_inspect(path, &bytes, limit),
        "macho_inspect" => macho_inspect(path, &bytes, args, limit),
        _ => unreachable!(),
    }
}

fn hex(value: u64) -> String {
    format!("0x{value:x}")
}

fn elf_flags(flags: u32) -> String {
    [(4, 'R'), (2, 'W'), (1, 'X')]
        .iter()
        .map(|&(bit, c)| if flags & bit != 0 { c } else { '-' })
        .collect()
}

fn elf_inspect(path: &Path, bytes: &[u8], limit: usize) -> Result<Value, String> {
    let elf = Elf::parse(bytes).map_err(|e| io_error("Invalid ELF image", e))?;
    let h = &elf.header;
    let segments: Vec<_> = elf.program_headers.iter().take(limit).map(|p| json!({
        "type":elf::program_header::pt_to_str(p.p_type),"flags":elf_flags(p.p_flags),"offset":p.p_offset,
        "virtualAddress":hex(p.p_vaddr),"fileSize":p.p_filesz,"memorySize":p.p_memsz,"align":p.p_align
    })).collect();
    let sections: Vec<_> = elf.section_headers.iter().take(limit).map(|s| json!({
        "name":elf.shdr_strtab.get_at(s.sh_name).unwrap_or(""),"type":elf::section_header::sht_to_str(s.sh_type),
        "address":hex(s.sh_addr),"offset":s.sh_offset,"sizeBytes":s.sh_size,"flags":hex(s.sh_flags)
    })).collect();
    let symbol = |sym: &elf::Sym, strtab: &goblin::strtab::Strtab| {
        json!({"name":strtab.get_at(sym.st_name).unwrap_or(""),"value":hex(sym.st_value),"sizeBytes":sym.st_size,
            "type":elf::sym::type_to_str(sym.st_type()),"bind":elf::sym::bind_to_str(sym.st_bind()),
            "defined":sym.st_shndx != 0})
    };
    let named = |sym: &elf::Sym, strtab: &goblin::strtab::Strtab| {
        strtab.get_at(sym.st_name).is_some_and(|n| !n.is_empty())
    };
    let imports: Vec<_> = elf
        .dynsyms
        .iter()
        .filter(|s| s.st_shndx == 0 && named(s, &elf.dynstrtab))
        .take(limit)
        .map(|s| symbol(&s, &elf.dynstrtab))
        .collect();
    let exports: Vec<_> = elf
        .dynsyms
        .iter()
        .filter(|s| {
            s.st_shndx != 0 && s.st_bind() != elf::sym::STB_LOCAL && named(s, &elf.dynstrtab)
        })
        .take(limit)
        .map(|s| symbol(&s, &elf.dynstrtab))
        .collect();
    let symbols: Vec<_> = elf
        .syms
        .iter()
        .filter(|s| named(s, &elf.strtab))
        .take(limit)
        .map(|s| symbol(&s, &elf.strtab))
        .collect();
    let build_id = elf
        .iter_note_sections(bytes, Some(".note.gnu.build-id"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .find(|n| n.n_type == elf::note::NT_GNU_BUILD_ID)
        .map(|n| {
            n.desc
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        });
    let truncated = elf.program_headers.len() > limit
        || elf.section_headers.len() > limit
        || imports.len() >= limit
        || exports.len() >= limit
        || symbols.len() >= limit;
    Ok(
        json!({"path":path.to_string_lossy(),"sizeBytes":bytes.len(),"is64Bit":elf.is_64,
        "endian":if elf.little_endian {"little"} else {"big"},"type":elf::header::et_to_str(h.e_type),
        "machine":elf::header::machine_to_str(h.e_machine),"machineId":h.e_machine,"entry":hex(elf.entry),
        "interpreter":elf.interpreter,"soname":elf.soname,"neededLibraries":elf.libraries,
        "runpaths":elf.runpaths,"rpaths":elf.rpaths,"buildId":build_id,
        "segmentCount":elf.program_headers.len(),"segments":segments,
        "sectionCount":elf.section_headers.len(),"sections":sections,
        "imports":imports,"exports":exports,"symbols":symbols,"truncated":truncated}),
    )
}

fn version_string(raw: u32) -> String {
    format!("{}.{}.{}", raw >> 16, (raw >> 8) & 0xff, raw & 0xff)
}

fn platform_name(platform: u32) -> &'static str {
    match platform {
        1 => "macos",
        2 => "ios",
        3 => "tvos",
        4 => "watchos",
        5 => "bridgeos",
        6 => "maccatalyst",
        7 => "ios_simulator",
        8 => "tvos_simulator",
        9 => "watchos_simulator",
        10 => "driverkit",
        11 => "visionos",
        12 => "visionos_simulator",
        _ => "unknown",
    }
}

fn arch_name(cputype: u32, cpusubtype: u32) -> &'static str {
    mach::constants::cputype::get_arch_name_from_types(cputype, cpusubtype).unwrap_or("unknown")
}

fn macho_value(macho: &MachO, limit: usize) -> Result<Value, String> {
    let h = &macho.header;
    let mut segments = Vec::new();
    for segment in macho.segments.iter().take(limit) {
        let sections: Vec<_> = segment
            .sections()
            .map_err(|e| io_error("Invalid Mach-O section", e))?
            .iter()
            .map(|(s, _)| {
                json!({"name":s.name().unwrap_or(""),"address":hex(s.addr),"sizeBytes":s.size,
                "offset":s.offset,"flags":format!("0x{:x}",s.flags)})
            })
            .collect();
        segments.push(
            json!({"name":segment.name().unwrap_or(""),"virtualAddress":hex(segment.vmaddr),
            "virtualSize":segment.vmsize,"fileOffset":segment.fileoff,"fileSize":segment.filesize,
            "maxProtection":segment.maxprot,"initProtection":segment.initprot,"sections":sections}),
        );
    }
    let mut uuid = None;
    let mut build = Value::Null;
    for command in &macho.load_commands {
        match &command.command {
            mach::load_command::CommandVariant::Uuid(u) => {
                let s: String = u.uuid.iter().map(|b| format!("{b:02X}")).collect();
                uuid = Some(format!(
                    "{}-{}-{}-{}-{}",
                    &s[..8],
                    &s[8..12],
                    &s[12..16],
                    &s[16..20],
                    &s[20..]
                ));
            }
            mach::load_command::CommandVariant::BuildVersion(b) => {
                build = json!({"platform":platform_name(b.platform),"minOs":version_string(b.minos),"sdk":version_string(b.sdk)});
            }
            _ => {}
        }
    }
    let exports: Vec<_> = macho
        .exports()
        .unwrap_or_default()
        .into_iter()
        .take(limit)
        .map(|e| json!({"name":e.name,"offset":hex(e.offset),"sizeBytes":e.size}))
        .collect();
    let imports: Vec<_> = macho
        .imports()
        .unwrap_or_default()
        .into_iter()
        .take(limit)
        .map(|i| json!({"name":i.name,"dylib":i.dylib,"lazy":i.is_lazy,"weak":i.is_weak,"address":hex(i.address)}))
        .collect();
    Ok(
        json!({"is64Bit":macho.is_64,"endian":if macho.little_endian {"little"} else {"big"},
        "cpu":arch_name(h.cputype, h.cpusubtype),"fileType":mach::header::filetype_to_str(h.filetype),
        "flags":format!("0x{:x}",h.flags),"loadCommandCount":h.ncmds,"entry":hex(macho.entry),"name":macho.name,
        "uuid":uuid,"buildVersion":build,"libraries":macho.libs.iter().filter(|l| **l != "self").collect::<Vec<_>>(),
        "rpaths":macho.rpaths,"segments":segments,"imports":imports,"exports":exports,
        "truncated":imports.len()>=limit || exports.len()>=limit}),
    )
}

fn macho_inspect(path: &Path, bytes: &[u8], args: &Value, limit: usize) -> Result<Value, String> {
    let mach = Mach::parse(bytes).map_err(|e| io_error("Invalid Mach-O image", e))?;
    match mach {
        Mach::Binary(macho) => {
            let mut value = macho_value(&macho, limit)?;
            value["path"] = json!(path.to_string_lossy());
            value["fat"] = json!(false);
            Ok(value)
        }
        Mach::Fat(fat) => {
            let arches = fat
                .arches()
                .map_err(|e| io_error("Invalid fat header", e))?;
            let index =
                optional_u64(args, "arch", 0, arches.len().saturating_sub(1) as u64)? as usize;
            let listed: Vec<_> = arches
                .iter()
                .map(|a| {
                    json!({"cpu":arch_name(a.cputype, a.cpusubtype),
                "offset":a.offset,"sizeBytes":a.size,"align":a.align})
                })
                .collect();
            let mut value = match fat
                .get(index)
                .map_err(|e| io_error("Invalid fat slice", e))?
            {
                SingleArch::MachO(macho) => macho_value(&macho, limit)?,
                SingleArch::Archive(_) => json!({"archive":true}),
            };
            value["path"] = json!(path.to_string_lossy());
            value["fat"] = json!(true);
            value["arches"] = json!(listed);
            value["selectedArch"] = json!(index);
            Ok(value)
        }
    }
}
