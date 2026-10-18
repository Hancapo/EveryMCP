# Proposed tools

Candidate additions to EveryMCP, chosen to fill gaps next to what the project already does well: binary/RE primitives, 3D math, and bounded host automation. Existing tools named here are the ones each proposal builds on.

## Tier 1: obvious gaps

| Tool | Purpose | Builds on |
| --- | --- | --- |
| `process_memory_read` | Read a bounded region of any Windows process (heap, other modules, private allocations) as hex or typed values. | `process_memory_map`, `binary_unpack` types |
| `pointer_chain_resolve` | Follow `[[base+off1]+off2]+…` through a live process and report every hop. | `address_translate`, `relative_target` |
| `process_pattern_scan` | Run `??`-wildcard byte patterns against a live module or region instead of a file. Byte signatures survive updates; file offsets don't. | `binary_pattern_search` syntax |
| `signature_generate` | Given a file plus an offset or RVA, build the shortest unique wildcarded signature. Wildcarding relative operands correctly needs an instruction decoder (`iced-x86`, pure Rust). | `binary_pattern_search` |
| `hash_compute` | Non-cryptographic hashes used by real formats: Jenkins one-at-a-time (JOAAT), FNV-1/1a 32/64, MurmurHash3, xxHash32/64, djb2, with optional case folding. | `crc_compute` |
| `hash_dictionary_match` | Hash a wordlist (inline or from a file) with one of those algorithms and report which known hashes it reverses. | `hash_compute` |
| `data_transform` | CyberChef-style chain: XOR (single- or multi-byte key), ROL/ROR, add/sub, base64/base32/hex, zlib/deflate/gzip/lz4. | `binary_strings`, `file_read_range` |

## Tier 2: deeper format coverage

| Tool | Purpose | Builds on |
| --- | --- | --- |
| `pe_debug_info` | Read the CodeView record (PDB path, GUID, age) and build the symbol-server URL, ready for `http_download_file`. | `pe_inspect` |
| `pe_resources` | Version info, manifest, and the resource tree with offsets. | `pe_inspect` |
| `pe_runtime_functions` | Function boundaries on x64 from `.pdata`/unwind data, without IDA. | `pe_address_map` |
| `elf_inspect` / `macho_inspect` | Bring ELF and Mach-O up to `pe_inspect`'s level. Header detection already exists in `inspect.rs::architecture`. | `executable_resolve` |
| `binary_carve` | Binwalk-style scan for embedded magic numbers (ZIP, PNG, DDS, zlib, gzip, PE, ELF) with offsets. | `binary_pattern_search` |
| `x86_disasm` / `x86_assemble` | Decode or encode short snippets with the same `iced-x86` dependency. Assembling patch bytes is the main draw, since IDA already disassembles well. | `x86_effective_address` |
| `struct_layout` | Take a C struct definition and return offsets, padding and `sizeof` for the target ABI; optionally decode a buffer through it. | `binary_unpack` |

## Tier 3: geometry and math

| Tool | Purpose | Builds on |
| --- | --- | --- |
| `bounding_volume` | AABB, OBB, or bounding sphere from a list of points. | `frustum_test`, `ray_primitive_intersect` |
| `mesh_inspect` | From vertices and indices: area, volume, normals, degenerate or duplicate triangles, watertightness. | `packed_vertex_decode` |
| `convex_hull` / `polygon_triangulate` | 2D and 3D hulls; polygon triangulation. | `polygon_measure_2d` |
| `expression_evaluate` | One arbitrary expression with variables, replacing chains of `add`/`multiply` calls. | basic arithmetic tools |
| `curve_fit` | Linear, polynomial or exponential least-squares fits with R². | `matrix_factor` |

## Windows automation

The README promises "Windows automation", but nothing can touch the GUI yet.

| Tool | Purpose |
| --- | --- |
| `window_list` | Window titles, PIDs, and rectangles. |
| `screen_capture` | Capture the screen, a window, or a region to PNG; pairs with `image_compare`. |
| `clipboard_get` / `clipboard_set` | Read and write the clipboard. |
| `registry_write` / `registry_list` | Write values and enumerate keys; today only single-value reads exist. |
| `wmi_query` | Run a generic WMI query. |

## Suggested first batch

Start with the four live-process tools (`process_memory_read`, `pointer_chain_resolve`, `process_pattern_scan`, `signature_generate`) plus `hash_compute` and `data_transform`. They reuse existing code (memory map, pattern syntax, unpack types, CRC). Together they make EveryMCP a good companion to IDA: IDA handles static analysis, and EveryMCP covers live-process and data-decoding work.

## Open question

The current process tools are strictly read-only ("does not alter the process"). Decide whether `process_memory_write` and in-memory patch tools should exist or whether read-only stays a firm rule. If they are added, keep them as separate, clearly labelled tools.
