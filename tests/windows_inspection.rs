mod common;
use common::{call, response};
use serde_json::json;

fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn dump_fixture() -> Vec<u8> {
    let mut bytes = vec![0; 744];
    bytes[..4].copy_from_slice(b"MDMP");
    put32(&mut bytes, 4, 0xa793);
    put32(&mut bytes, 8, 4);
    put32(&mut bytes, 12, 32);
    for (index, (kind, size, rva)) in [(7, 56, 80), (3, 52, 136), (4, 112, 188), (6, 168, 300)]
        .into_iter()
        .enumerate()
    {
        put32(&mut bytes, 32 + index * 12, kind);
        put32(&mut bytes, 36 + index * 12, size);
        put32(&mut bytes, 40 + index * 12, rva);
    }
    bytes[80] = 9; // AMD64
    bytes[86] = 8;
    put32(&mut bytes, 88, 10);
    put32(&mut bytes, 96, 26100);
    put32(&mut bytes, 136, 1);
    put32(&mut bytes, 140, 42);
    put32(&mut bytes, 180, 256);
    put32(&mut bytes, 184, 468);
    put32(&mut bytes, 188, 1);
    put64(&mut bytes, 192, 0x140000000);
    put32(&mut bytes, 200, 8192);
    put32(&mut bytes, 212, 724);
    put32(&mut bytes, 300, 42);
    put32(&mut bytes, 308, 0xc0000005);
    put64(&mut bytes, 324, 0x140001234);
    put32(&mut bytes, 332, 2);
    put64(&mut bytes, 340, 1);
    put64(&mut bytes, 348, 0xdeadbeef);
    put32(&mut bytes, 460, 256);
    put32(&mut bytes, 464, 468);
    put32(&mut bytes, 516, 0x100003);
    put64(&mut bytes, 588, 0x1122334455667788);
    put64(&mut bytes, 620, 0x7ff000);
    put64(&mut bytes, 716, 0x140001234);
    put32(&mut bytes, 724, 16);
    for (index, value) in "test.dll".encode_utf16().enumerate() {
        bytes[728 + index * 2..730 + index * 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[test]
fn minidump_reports_exception_context_and_modules() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("fixture.dmp");
    std::fs::write(&path, dump_fixture()).unwrap();
    let result = call("minidump_inspect", json!({"path":path,"limit":1}));
    assert_eq!(result["streamCount"], 4);
    assert_eq!(result["streams"].as_array().unwrap().len(), 1);
    assert_eq!(result["streamsTruncated"], true);
    assert_eq!(result["systemInfo"]["architecture"], "amd64");
    assert_eq!(result["systemInfo"]["buildNumber"], 26100);
    assert_eq!(result["modules"][0]["name"], "test.dll");
    assert_eq!(result["modules"][0]["baseAddress"], "0x140000000");
    assert_eq!(result["threads"][0]["threadId"], 42);
    assert_eq!(result["exception"]["code"], "0xc0000005");
    assert_eq!(
        result["exception"]["parameters"],
        json!(["0x1", "0xdeadbeef"])
    );
    assert_eq!(
        result["exception"]["context"]["registers"]["rip"],
        "0x140001234"
    );
    assert_eq!(
        result["exception"]["context"]["registers"]["rax"],
        "0x1122334455667788"
    );
    assert_eq!(
        result["threads"][0]["context"]["registers"]["rsp"],
        "0x7ff000"
    );
}

#[test]
fn minidump_rejects_invalid_offsets_counts_and_strings() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("invalid.dmp");
    for (at, value) in [
        (12, u32::MAX),
        (40, 740),
        (136, u32::MAX),
        (184, 740),
        (212, 743),
        (724, 15),
    ] {
        let mut bytes = dump_fixture();
        put32(&mut bytes, at, value);
        std::fs::write(&path, bytes).unwrap();
        let result = response("minidump_inspect", json!({"path":path}));
        assert_eq!(result["result"]["isError"], true, "offset {at}: {result}");
        assert!(!result.to_string().contains("Unknown"), "{result}");
    }
}

#[test]
fn minidump_omits_register_groups_not_present_in_context() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("flags.dmp");
    let mut bytes = dump_fixture();
    put32(&mut bytes, 516, 0x100001); // Control, no integer registers.
    std::fs::write(&path, bytes).unwrap();
    let result = call("minidump_inspect", json!({"path":path}));
    assert_eq!(
        result["exception"]["context"]["registers"]["rip"],
        "0x140001234"
    );
    assert!(
        result["exception"]["context"]["registers"]
            .get("rax")
            .is_none()
    );
}

#[cfg(windows)]
#[test]
fn process_connections_finds_local_tcp_and_udp_fixtures() {
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let tcp6 = std::net::TcpListener::bind("[::1]:0").unwrap();
    let udp6 = std::net::UdpSocket::bind("[::1]:0").unwrap();
    let peer = std::net::TcpStream::connect(tcp.local_addr().unwrap()).unwrap();
    let (_accepted, _) = tcp.accept().unwrap();
    let result = call("process_connections", json!({"pid":std::process::id()}));
    let endpoints = result["connections"].as_array().unwrap();
    assert!(endpoints.iter().any(|v| v["protocol"] == "tcp"
        && v["localPort"] == tcp.local_addr().unwrap().port()
        && v["state"] == "Listen"));
    assert!(
        endpoints
            .iter()
            .any(|v| v["protocol"] == "udp" && v["localPort"] == udp.local_addr().unwrap().port())
    );
    assert!(endpoints.iter().any(|v| v["protocol"] == "tcp"
        && v["addressFamily"] == "IPv6"
        && v["localPort"] == tcp6.local_addr().unwrap().port()));
    assert!(endpoints.iter().any(|v| v["protocol"] == "udp"
        && v["addressFamily"] == "IPv6"
        && v["localPort"] == udp6.local_addr().unwrap().port()));
    assert!(endpoints.iter().any(|v| v["protocol"] == "tcp"
        && v["state"] == "Established"
        && v["localPort"] == peer.local_addr().unwrap().port()
        && v["remotePort"] == tcp.local_addr().unwrap().port()));
    assert!(endpoints.iter().all(|v| v["pid"] == std::process::id()));
    let limited = call(
        "process_connections",
        json!({"pid":std::process::id(),"limit":1}),
    );
    assert_eq!(limited["connections"].as_array().unwrap().len(), 1);
    assert_eq!(limited["truncated"], true);
}

#[cfg(windows)]
#[test]
fn process_memory_map_reports_query_metadata_and_paginates() {
    let result = call(
        "process_memory_map",
        json!({"pid":std::process::id(),"limit":2000}),
    );
    let regions = result["regions"].as_array().unwrap();
    assert!(regions.iter().any(|v| v["state"] == "MEM_COMMIT"
        && v["type"] == "MEM_IMAGE"
        && v["module"]["path"].is_string()));
    let limited = call(
        "process_memory_map",
        json!({"pid":std::process::id(),"limit":1}),
    );
    assert_eq!(limited["regions"].as_array().unwrap().len(), 1);
    assert_eq!(limited["nextOffset"], 1);
    let next = call(
        "process_memory_map",
        json!({"pid":std::process::id(),"limit":1,"offset":1}),
    );
    assert_ne!(
        limited["regions"][0]["baseAddress"],
        next["regions"][0]["baseAddress"]
    );
    assert!(next["regions"][0]["sizeBytes"].as_u64().unwrap() > 0);
}

#[test]
fn inspection_limits_and_pid_are_validated() {
    for (name, args) in [
        ("process_connections", json!({"pid":0})),
        ("process_connections", json!({"pid":1,"limit":2001})),
        ("process_memory_map", json!({"pid":1,"limit":0})),
        (
            "process_memory_map",
            json!({"pid":std::process::id(),"offset":100000}),
        ),
        (
            "minidump_inspect",
            json!({"path":"missing.dmp","limit":1001}),
        ),
    ] {
        let result = response(name, args);
        assert_eq!(result["result"]["isError"], true, "{result}");
        assert!(!result.to_string().contains("Unknown"), "{result}");
    }
}

#[cfg(windows)]
#[test]
fn process_inspection_rejects_a_missing_pid() {
    for name in ["process_connections", "process_memory_map"] {
        let result = response(name, json!({"pid":2147483647}));
        assert_eq!(result["result"]["isError"], true, "{result}");
        assert!(!result.to_string().contains("Unknown"), "{result}");
    }
}
