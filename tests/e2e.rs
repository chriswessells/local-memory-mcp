mod common;

use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::time::timeout;

struct ServerProcess {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

fn spawn_server(base_dir: &Path) -> ServerProcess {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_local-memory-mcp"))
        .env("LOCAL_MEMORY_HOME", base_dir)
        .env("RUST_LOG", "info")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn server");

    let stdin = Some(BufWriter::new(child.stdin.take().unwrap()));
    let stdout = BufReader::new(child.stdout.take().unwrap());
    ServerProcess {
        child,
        stdin,
        stdout,
    }
}

async fn send_request(proc: &mut ServerProcess, id: u64, method: &str, params: Value) {
    let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    let line = serde_json::to_string(&msg).unwrap() + "\n";
    let stdin = proc.stdin.as_mut().unwrap();
    stdin.write_all(line.as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();
}

async fn send_notification(proc: &mut ServerProcess, method: &str) {
    let msg = json!({"jsonrpc": "2.0", "method": method});
    let line = serde_json::to_string(&msg).unwrap() + "\n";
    let stdin = proc.stdin.as_mut().unwrap();
    stdin.write_all(line.as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();
}

async fn read_response(proc: &mut ServerProcess, expected_id: u64) -> Value {
    for _ in 0..100 {
        let mut line = String::new();
        let n = timeout(Duration::from_secs(5), proc.stdout.read_line(&mut line))
            .await
            .expect("timeout reading response")
            .expect("IO error reading response");
        assert!(n > 0, "server closed stdout unexpectedly");
        let v: Value = serde_json::from_str(line.trim()).expect("invalid JSON from server");
        // Skip notifications (no id field)
        if let Some(id) = v.get("id") {
            if id.as_u64() == Some(expected_id) {
                return v;
            }
        }
    }
    panic!("read_response: exceeded 100 lines without finding response id={expected_id}");
}

async fn mcp_initialize(proc: &mut ServerProcess) -> Value {
    send_request(
        proc,
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0.1.0"}
        }),
    )
    .await;
    let resp = read_response(proc, 1).await;
    send_notification(proc, "notifications/initialized").await;
    resp
}

fn tool_call_params(name: &str, args: Value) -> Value {
    json!({"name": name, "arguments": args})
}

fn extract_tool_text(resp: &Value) -> Value {
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn test_e2e_mcp_lifecycle() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut proc = spawn_server(dir.path());

    // Initialize
    let init = mcp_initialize(&mut proc).await;
    assert!(init["result"]["capabilities"].is_object());

    // tools/list
    send_request(&mut proc, 2, "tools/list", json!({})).await;
    let resp = read_response(&mut proc, 2).await;
    let tools = resp["result"]["tools"].as_array().unwrap();
    assert!(
        tools.len() >= 22,
        "expected at least 22 tools, got {}",
        tools.len()
    );
    // Spot-check memory.create_event has actor_id required
    let add_event_tool = tools
        .iter()
        .find(|t| t["name"] == "memory.create_event")
        .unwrap();
    let required = add_event_tool["inputSchema"]["required"]
        .as_array()
        .unwrap();
    assert!(required.iter().any(|r| r == "actor_id"));

    // tools/call: memory.create_event
    send_request(
        &mut proc,
        3,
        "tools/call",
        tool_call_params(
            "memory.create_event",
            json!({
                "actor_id": "a1", "session_id": "s1", "event_type": "conversation",
                "role": "user", "content": "e2e test"
            }),
        ),
    )
    .await;
    let resp = read_response(&mut proc, 3).await;
    let ev = extract_tool_text(&resp);
    assert!(ev["id"].is_string());

    // tools/call: memory.create_memory_record
    send_request(
        &mut proc,
        4,
        "tools/call",
        tool_call_params(
            "memory.create_memory_record",
            json!({
                "actor_id": "a1", "content": "e2e memory", "strategy": "core"
            }),
        ),
    )
    .await;
    let resp = read_response(&mut proc, 4).await;
    let mem = extract_tool_text(&resp);
    let mem_id = mem["id"].as_str().unwrap().to_string();

    // tools/call: memory.retrieve_memory_records
    send_request(
        &mut proc,
        5,
        "tools/call",
        tool_call_params(
            "memory.retrieve_memory_records",
            json!({
                "actor_id": "a1", "search_query": "e2e"
            }),
        ),
    )
    .await;
    let resp = read_response(&mut proc, 5).await;
    let results = extract_tool_text(&resp);
    assert!(!results.as_array().unwrap().is_empty());

    // tools/call: graph.create_edge (need two memories)
    send_request(
        &mut proc,
        6,
        "tools/call",
        tool_call_params(
            "memory.create_memory_record",
            json!({
                "actor_id": "a1", "content": "second memory", "strategy": "core"
            }),
        ),
    )
    .await;
    let resp = read_response(&mut proc, 6).await;
    let mem2_id = extract_tool_text(&resp)["id"].as_str().unwrap().to_string();

    send_request(
        &mut proc,
        7,
        "tools/call",
        tool_call_params(
            "graph.create_edge",
            json!({
                "actor_id": "a1", "from_memory_record_id": mem_id, "to_memory_record_id": mem2_id, "label": "test"
            }),
        ),
    )
    .await;
    let resp = read_response(&mut proc, 7).await;
    let edge = extract_tool_text(&resp);
    assert!(edge["id"].is_string());

    // tools/call with invalid params → isError: true
    send_request(
        &mut proc,
        8,
        "tools/call",
        tool_call_params(
            "memory.create_event",
            json!({
                "actor_id": "", "session_id": "s1", "event_type": "conversation",
                "role": "user", "content": "bad"
            }),
        ),
    )
    .await;
    let resp = read_response(&mut proc, 8).await;
    assert_eq!(resp["result"]["isError"], true);

    // Close stdin → server should exit cleanly
    drop(proc.stdin.take());
    let status = timeout(Duration::from_secs(5), proc.child.wait())
        .await
        .expect("timeout waiting for exit")
        .expect("wait failed");
    assert!(status.success());
}

#[tokio::test]
async fn test_e2e_stderr_logging() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut proc = spawn_server(dir.path());

    // Grab stderr handle before initialize
    let mut stderr = BufReader::new(proc.child.stderr.take().unwrap());
    let stderr_lines = tokio::spawn(async move {
        let mut lines = Vec::new();
        let mut line = String::new();
        while let Ok(Ok(n)) = timeout(Duration::from_secs(3), stderr.read_line(&mut line)).await {
            if n == 0 {
                break;
            }
            lines.push(std::mem::take(&mut line));
        }
        lines
    });

    // Initialize + one tool call
    mcp_initialize(&mut proc).await;
    send_request(
        &mut proc,
        2,
        "tools/call",
        tool_call_params(
            "memory.create_memory_record",
            json!({
                "actor_id": "a1", "content": "log test", "strategy": "core"
            }),
        ),
    )
    .await;
    let resp = read_response(&mut proc, 2).await;
    // Verify stdout is valid JSON
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(serde_json::from_str::<Value>(text).is_ok());

    // Shutdown
    drop(proc.stdin.take());
    let _ = timeout(Duration::from_secs(5), proc.child.wait()).await;

    // Verify stderr has logging output
    let lines = stderr_lines.await.unwrap();
    assert!(!lines.is_empty(), "expected tracing output on stderr");
}
