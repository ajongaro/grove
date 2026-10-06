//! Speaks JSON-RPC to the real `grove mcp` binary over stdio.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn mcp_lists_five_tools_and_answers_a_call() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join("projects")).unwrap();
    std::fs::write(
        home.join("projects/demo.toml"),
        "name = \"demo\"\npath = \"/tmp/demo\"\n",
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_grove"))
        .arg("mcp")
        .env("GROVE_HOME", &home)
        .env("GROVE_PROJECT", "demo")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut send = |v: serde_json::Value| writeln!(stdin, "{v}").unwrap();
    let mut recv = || {
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        serde_json::from_str::<serde_json::Value>(&line).unwrap()
    };

    send(
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}),
    );
    let init = recv();
    assert!(init["result"]["serverInfo"].is_object(), "{init}");
    send(serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}));

    send(serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
    let list = recv();
    let mut names: Vec<String> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "get_project",
            "list_projects",
            "read_wiki",
            "update_project",
            "write_wiki_section"
        ]
    );

    send(
        serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_project","arguments":{}}}),
    );
    let call = recv();
    let text = call["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Name: demo"), "{call}");

    drop(stdin);
    child.wait().unwrap();
}
