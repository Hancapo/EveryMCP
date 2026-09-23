use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

pub fn call(name: &str, arguments: Value) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_EveryMCP"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}});
    writeln!(child.stdin.take().unwrap(), "{request}").unwrap();
    let response: Value = serde_json::from_str(
        &BufReader::new(child.stdout.take().unwrap())
            .lines()
            .next()
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(response["result"]["isError"], false, "{response}");
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
