mod common;

use common::{call, response};
use serde_json::{Value, json};

fn layout(source: &str, extra: Value) -> Value {
    let mut args = json!({"source":source});
    args.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    call("struct_layout", args)
}

fn offsets(result: &Value) -> Vec<(String, u64)> {
    result["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["name"].as_str().unwrap().to_owned(),
                f["offset"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn named(pairs: &[(&str, u64)]) -> Vec<(String, u64)> {
    pairs.iter().map(|&(n, o)| (n.to_owned(), o)).collect()
}

// Expected values below were confirmed with clang for each target triple.

#[test]
fn abis_differ_in_long_pointer_and_double_alignment() {
    let source = "struct Mixed { char a; double b; short c; long d; long long e; void *p; };";
    // long is 4 bytes on Windows and 8 on 64-bit Linux; i386 Linux aligns double and
    // long long to 4 inside structs.
    let cases = [
        ("win64", 40, 8, [0, 8, 16, 20, 24, 32]),
        ("win32", 40, 8, [0, 8, 16, 20, 24, 32]),
        ("sysv64", 48, 8, [0, 8, 16, 24, 32, 40]),
        ("sysv32", 32, 4, [0, 4, 12, 16, 20, 28]),
    ];
    for (abi, size, align, fields) in cases {
        let result = layout(source, json!({"abi":abi}));
        let got: Vec<u64> = offsets(&result).iter().map(|(_, o)| *o).collect();
        assert_eq!(got, fields, "{abi}");
        assert_eq!(result["size"], size, "{abi}");
        assert_eq!(result["align"], align, "{abi}");
    }
}

#[test]
fn nested_anonymous_and_union_members_are_flattened() {
    let source = "
        #define NAME_LEN (8 * 2)
        struct Vec3 { float x, y, z; };
        struct WithUnion { char kind; union { int i; double d; }; struct { short a, b; } pair; };
        struct Nested { struct Vec3 pos; char tag; struct Vec3 vel[2]; char name[NAME_LEN]; };";
    let result = layout(source, json!({"name":"WithUnion"}));
    assert_eq!(
        offsets(&result),
        named(&[
            ("kind", 0),
            ("i", 8),
            ("d", 8),
            ("pair", 16),
            ("pair.a", 16),
            ("pair.b", 18)
        ])
    );
    assert_eq!(result["size"], 24);
    assert_eq!(
        result["padding"],
        json!([{"offset":1,"offsetHex":"0x1","size":7},{"offset":20,"offsetHex":"0x14","size":4}])
    );
    let nested = layout(source, json!({}));
    assert_eq!(nested["name"], "Nested");
    assert_eq!(nested["size"], 56);
    assert_eq!(
        offsets(&nested)[..5],
        named(&[
            ("pos", 0),
            ("pos.x", 0),
            ("pos.y", 4),
            ("pos.z", 8),
            ("tag", 12)
        ])[..]
    );
    let vel = &nested["fields"][5];
    assert_eq!(
        (
            vel["name"].as_str(),
            vel["offset"].as_u64(),
            vel["count"].as_u64()
        ),
        (Some("vel"), Some(16), Some(2))
    );
}

#[test]
fn bitfields_follow_msvc_and_gcc_rules() {
    let source = "struct Bits2 { char a; int b : 4; char c; long long d : 40; };";
    let bits = |abi: &str| -> (u64, Vec<(String, u64, u64)>) {
        let result = layout(source, json!({"abi":abi}));
        let fields = result["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let bit = f["offset"].as_u64().unwrap() * 8 + f["bitOffset"].as_u64().unwrap_or(0);
                (
                    f["name"].as_str().unwrap().to_owned(),
                    bit,
                    f["bitWidth"].as_u64().unwrap_or(0),
                )
            })
            .collect();
        (result["size"].as_u64().unwrap(), fields)
    };
    let owned = |v: &[(&str, u64, u64)]| {
        v.iter()
            .map(|&(n, b, w)| (n.to_owned(), b, w))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        bits("win64"),
        (
            24,
            owned(&[("a", 0, 0), ("b", 32, 4), ("c", 64, 0), ("d", 128, 40)])
        )
    );
    assert_eq!(
        bits("sysv64"),
        (
            8,
            owned(&[("a", 0, 0), ("b", 8, 4), ("c", 16, 0), ("d", 24, 40)])
        )
    );
    assert_eq!(
        bits("sysv32"),
        (
            8,
            owned(&[("a", 0, 0), ("b", 8, 4), ("c", 16, 0), ("d", 24, 40)])
        )
    );
    let zero = "struct Bits3 { unsigned char a : 3; unsigned char b : 7; unsigned int : 0; short c : 9; };";
    assert_eq!(layout(zero, json!({"abi":"win64"}))["size"], 8);
    assert_eq!(layout(zero, json!({"abi":"sysv64"}))["size"], 6);
}

#[test]
fn packing_and_alignment_controls() {
    let source = "
        #pragma pack(push, 1)
        struct Packed { char a; int b; short c; double d; };
        #pragma pack(pop)
        struct __attribute__((aligned(32))) Wide { int x; char y; };
        struct Aligned { char a; _Alignas(16) int b; char c; };
        struct Plain { char a; int b; };";
    let packed = layout(source, json!({"name":"Packed"}));
    assert_eq!(
        (packed["size"].as_u64(), packed["align"].as_u64()),
        (Some(15), Some(1))
    );
    assert_eq!(packed["paddingBytes"], 0);
    assert_eq!(layout(source, json!({"name":"Wide"}))["size"], 32);
    let aligned = layout(source, json!({"name":"Aligned"}));
    assert_eq!(
        (aligned["size"].as_u64(), offsets(&aligned)[1].1),
        (Some(32), 16)
    );
    assert_eq!(layout(source, json!({"name":"Plain","pack":2}))["size"], 6);
}

#[test]
fn decodes_bytes_through_the_layout() {
    let source = "
        #pragma pack(push, 1)
        struct Entry { char name[8]; wchar_t wide[4]; unsigned flags : 3; unsigned level : 5;
            int delta : 8; bool on; float scale; double weight; void *link; unsigned char raw[3]; };
        #pragma pack(pop)";
    let mut data = b"Hello\0\0\0".to_vec();
    data.extend("Hi!\0".encode_utf16().flat_map(u16::to_le_bytes));
    data.extend((0b1111_1101_u32 | (0xF0 << 8)).to_le_bytes());
    data.push(1);
    data.extend(1.5f32.to_le_bytes());
    data.extend((-2.25f64).to_le_bytes());
    data.extend(0x14_00AB_CDEFu64.to_le_bytes());
    data.extend([0xAA, 0xBB, 0xCC]);
    let hex: String = data.iter().map(|b| format!("{b:02X}")).collect();
    let values = &layout(source, json!({"data":hex}))["values"];
    assert_eq!(
        *values,
        json!({"name":"Hello","wide":"Hi!","flags":"5","level":"31","delta":"-16","on":true,
            "scale":1.5,"weight":-2.25,"link":"0x1400ABCDEF","raw":"AA BB CC"})
    );
    let short = response("struct_layout", json!({"source":source,"data":"00"}));
    assert_eq!(short["result"]["isError"], true);
}

#[cfg(windows)]
#[test]
fn decodes_a_pe_header_from_a_file_offset() {
    let source = "
        typedef struct _IMAGE_FILE_HEADER { WORD Machine; WORD NumberOfSections; DWORD TimeDateStamp;
            DWORD PointerToSymbolTable; DWORD NumberOfSymbols; WORD SizeOfOptionalHeader;
            WORD Characteristics; } IMAGE_FILE_HEADER;
        typedef struct { DWORD Signature; IMAGE_FILE_HEADER FileHeader; } NT_PREFIX;
        typedef struct { WORD e_magic; WORD e_unused[29]; LONG e_lfanew; } DOS;";
    let path = env!("CARGO_BIN_EXE_EveryMCP");
    let dos = layout(source, json!({"name":"DOS","path":path}));
    assert_eq!(dos["size"], 64);
    assert_eq!(dos["values"]["e_magic"], "23117");
    let nt = layout(
        source,
        json!({"name":"NT_PREFIX","path":path,"offset":dos["values"]["e_lfanew"]}),
    );
    assert_eq!(nt["values"]["Signature"], "17744"); // "PE\0\0"
    assert_eq!(nt["values"]["FileHeader.Machine"], "34404"); // AMD64
}

#[test]
fn reports_clear_errors() {
    for source in [
        "struct A { int x }",
        "struct A { struct Missing m; };",
        "struct A { int x : 40; };",
        "struct A { int x[N]; };",
        "int x;",
    ] {
        assert_eq!(
            response("struct_layout", json!({"source":source}))["result"]["isError"],
            true,
            "{source}"
        );
    }
    assert_eq!(
        response(
            "struct_layout",
            json!({"source":"struct A { int x; };","abi":"arm"})
        )["result"]["isError"],
        true
    );
}
