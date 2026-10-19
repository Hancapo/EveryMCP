#![cfg(windows)]

mod common;

use common::{call, response};
use serde_json::json;

#[test]
fn window_list_reports_consistent_entries() {
    let result = call(
        "window_list",
        json!({"includeHidden":true,"includeUntitled":true,"limit":5}),
    );
    let windows = result["windows"].as_array().unwrap();
    assert!(!windows.is_empty());
    assert!(windows.len() <= 5);
    assert_eq!(result["truncated"], result["matched"].as_u64().unwrap() > 5);
    for window in windows {
        assert!(window["hwnd"].as_str().unwrap().starts_with("0x"));
        assert!(window["pid"].is_u64());
        assert!(window["rect"]["width"].is_i64());
    }
    let none = call(
        "window_list",
        json!({"title":"no window has this title 7f3a9c"}),
    );
    assert_eq!(none["matched"], 0);
}

#[test]
fn screen_capture_writes_region_png_and_refuses_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shot.png");
    let screen = call(
        "window_list",
        json!({"limit":1,"includeHidden":true,"includeUntitled":true}),
    )["virtualScreen"]
        .clone();
    let (x, y) = (screen["x"].as_i64().unwrap(), screen["y"].as_i64().unwrap());
    let args = json!({"outputPath":path,"x":x,"y":y,"width":32,"height":16});
    let result = call("screen_capture", args.clone());
    assert_eq!(
        (result["width"].as_u64(), result["height"].as_u64()),
        (Some(32), Some(16))
    );
    assert_eq!(result["source"], "screen");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(result["bytesWritten"], bytes.len());
    assert_eq!(response("screen_capture", args)["result"]["isError"], true);
}

#[test]
fn screen_capture_validates_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shot.png");
    for args in [
        json!({"outputPath":path,"x":0,"y":0,"width":10}),
        json!({"outputPath":path,"x":0,"y":0,"width":0,"height":10}),
        json!({"outputPath":path,"hwnd":"0x1","x":0,"y":0,"width":1,"height":1}),
        json!({"outputPath":path,"hwnd":"not a handle"}),
        json!({"outputPath":path,"method":"gpu"}),
    ] {
        assert_eq!(response("screen_capture", args)["result"]["isError"], true);
    }
    assert!(!path.exists());
}

/// Replaces the user's clipboard, so it only runs on request:
/// `cargo test --test desktop_tools -- --ignored`.
#[test]
#[ignore]
fn clipboard_round_trips_unicode_text() {
    let previous = call("clipboard_get", json!({}))["text"].clone();
    let text = "EveryMCP clipboard ✓ ünïcode\r\nsecond line";
    let set = call("clipboard_set", json!({"text":text}));
    assert_eq!(set["chars"], text.chars().count());
    let read = call("clipboard_get", json!({"maxChars":8}));
    assert_eq!(read["text"], "EveryMCP");
    assert_eq!(read["textChars"], text.chars().count());
    assert_eq!(read["textTruncated"], true);
    assert!(
        read["formats"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "CF_UNICODETEXT")
    );
    assert_eq!(call("clipboard_get", json!({}))["text"], text);
    if let Some(previous) = previous.as_str() {
        call("clipboard_set", json!({"text":previous}));
    }
}
