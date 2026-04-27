#![allow(dead_code)]

use local_memory_mcp::store::StoreManager;
use local_memory_mcp::tools::MemoryServer;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

pub fn setup() -> (TempDir, MemoryServer) {
    let dir = TempDir::new().unwrap();
    let mut mgr = StoreManager::with_base_dir(dir.path().to_path_buf()).unwrap();
    mgr.open_default().unwrap();
    let server = MemoryServer::new(Arc::new(Mutex::new(mgr)));
    (dir, server)
}

pub fn parse_ok(result: Result<String, String>) -> serde_json::Value {
    let s = result.expect("expected Ok response");
    serde_json::from_str(&s).expect("response is not valid JSON")
}

pub fn parse_err(result: Result<String, String>, expected_code: &str) -> serde_json::Value {
    let s = result.expect_err("expected Err response");
    let v: serde_json::Value = serde_json::from_str(&s).expect("error is not valid JSON");
    assert_eq!(v["code"].as_str().unwrap(), expected_code);
    v
}
