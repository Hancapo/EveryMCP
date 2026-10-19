mod common;

use common::{call, response};
use flate2::Compression;
use flate2::write::{GzEncoder, ZlibEncoder};
use serde_json::{Value, json};
use std::io::Write;

fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(5, 3, image::Rgba([10, 20, 30, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn zip() -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, text) in [
        ("a.txt", "alpha ".repeat(50)),
        ("b.txt", "beta ".repeat(20)),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn compressed(gzip: bool) -> Vec<u8> {
    let payload = b"carved payload ".repeat(30);
    if gzip {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&payload).unwrap();
        encoder.finish().unwrap()
    } else {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&payload).unwrap();
        encoder.finish().unwrap()
    }
}

/// A 16x16 DXT1 texture with one mip level: 128-byte header plus 128 bytes of blocks.
fn dds() -> Vec<u8> {
    let mut f = b"DDS ".to_vec();
    for v in [124u32, 0x1007, 16, 16, 0, 0, 1] {
        f.extend_from_slice(&v.to_le_bytes());
    }
    f.resize(76, 0);
    f.extend_from_slice(&32u32.to_le_bytes());
    f.extend_from_slice(&4u32.to_le_bytes());
    f.extend_from_slice(b"DXT1");
    f.resize(128, 0);
    f.extend(std::iter::repeat_n(0x55, 128));
    f
}

/// Concatenates parts separated by filler and returns the blob plus each part's span.
fn blob(parts: &[(&'static str, Vec<u8>)]) -> (Vec<u8>, Vec<(&'static str, u64, u64)>) {
    let mut data = b"filler MZ 78 9C PK".repeat(20);
    let mut spans = Vec::new();
    for (kind, bytes) in parts {
        spans.push((*kind, data.len() as u64, bytes.len() as u64));
        data.extend_from_slice(bytes);
        data.extend_from_slice(&[0xEE; 37]);
    }
    (data, spans)
}

fn summary(result: &Value) -> Vec<(String, u64, u64)> {
    result["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["type"].as_str().unwrap().to_owned(),
                m["offset"].as_u64().unwrap(),
                m["sizeBytes"].as_u64().unwrap_or(0),
            )
        })
        .collect()
}

fn expected(spans: &[(&str, u64, u64)]) -> Vec<(String, u64, u64)> {
    spans
        .iter()
        .map(|&(kind, offset, size)| (kind.to_owned(), offset, size))
        .collect()
}

#[test]
fn carve_finds_and_sizes_embedded_files() {
    let mut parts = vec![
        ("png", png()),
        ("zip", zip()),
        ("gzip", compressed(true)),
        ("zlib", compressed(false)),
        ("dds", dds()),
    ];
    if cfg!(windows) {
        parts.push(("pe", std::fs::read(env!("CARGO_BIN_EXE_EveryMCP")).unwrap()));
    }
    let (data, spans) = blob(&parts);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blob.bin");
    std::fs::write(&path, &data).unwrap();
    let result = call("binary_carve", json!({"path":path}));
    // The executable may itself embed compressed data; check the parts we placed.
    let found = summary(&result);
    for span in expected(&spans) {
        assert!(found.contains(&span), "{span:?} missing from {found:?}");
    }
    assert_eq!(result["matches"][0]["details"]["width"], 5);
    assert_eq!(result["limitReached"], false);
    assert_eq!(result["scannedBytes"], data.len());

    // Without skipContained the PNG's own zlib stream is reported too.
    let nested = call("binary_carve", json!({"path":path,"skipContained":false}));
    let png_start = spans[0].1;
    assert!(
        summary(&nested)
            .iter()
            .any(|(kind, offset, _)| kind == "zlib"
                && *offset > png_start
                && *offset < png_start + spans[0].2)
    );

    let only = call("binary_carve", json!({"path":path,"types":["zip","dds"]}));
    assert_eq!(summary(&only), expected(&[spans[1], spans[4]]));
}

#[test]
fn carve_pages_and_bounds_the_scan() {
    let (data, spans) = blob(&[
        ("gzip", compressed(true)),
        ("zlib", compressed(false)),
        ("dds", dds()),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blob.bin");
    std::fs::write(&path, &data).unwrap();
    let first = call("binary_carve", json!({"path":path,"limit":1}));
    assert_eq!(summary(&first), expected(&spans[..1]));
    assert_eq!(first["limitReached"], true);
    assert_eq!(first["nextOffset"], spans[1].1);
    let rest = call(
        "binary_carve",
        json!({"path":path,"offset":first["nextOffset"]}),
    );
    assert_eq!(summary(&rest), expected(&spans[1..]));
    let window = call(
        "binary_carve",
        json!({"path":path,"offset":spans[1].1,"maxScanBytes":spans[2].1 - spans[1].1}),
    );
    assert_eq!(summary(&window), expected(&spans[1..2]));
    assert_eq!(window["nextOffset"], spans[2].1);
    assert_eq!(
        response("binary_carve", json!({"path":path,"types":["jpeg"]}))["result"]["isError"],
        true
    );
}

#[test]
fn carve_rejects_corrupt_streams() {
    let mut gzip = compressed(true);
    let middle = gzip.len() / 2;
    gzip[middle] ^= 0xFF;
    let mut zlib = compressed(false);
    let end = zlib.len() - 1;
    zlib[end] ^= 0xFF; // breaks the Adler-32 checksum
    let mut png = png();
    png.truncate(png.len() - 6); // IEND cut off
    let (data, spans) = blob(&[("gzip", gzip), ("zlib", zlib), ("png", png)]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blob.bin");
    std::fs::write(&path, &data).unwrap();
    let result = call("binary_carve", json!({"path":path,"skipContained":false}));
    // Only the truncated PNG's intact IDAT zlib stream survives; the three broken
    // items themselves are rejected.
    let found = summary(&result);
    let png = spans[2];
    assert!(
        found
            .iter()
            .all(|(kind, offset, _)| kind == "zlib" && *offset > png.1 && *offset < png.1 + png.2),
        "{found:?}"
    );
}
