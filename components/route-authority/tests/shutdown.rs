#![cfg(unix)]
use std::{
    io::Write,
    net::{TcpListener, TcpStream},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
#[test]
fn sigterm_bounds_partial_header_and_body_drain() {
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let config=serde_json::json!({"targets":[{"name":"room","idle_route":"idle","driver_target":"group"}],"adapters":[{"token":"adapter","policy":{"source":"source","priority":1,"targets":["room"],"routes":["music"],"min_ttl_ms":1,"max_ttl_ms":1000}}],"reporters":[{"token":"reporter","policy":{"reporter":"presence","targets":["room"]}}],"reader_tokens":["read"],"admin_tokens":["admin"],"snapcast_url":"http://127.0.0.1:9/jsonrpc"}).to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_household-audio-router"))
        .env("PORT", port.to_string())
        .env("ROUTER_CONFIG_JSON", config)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let address = format!("127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut header = loop {
        match TcpStream::connect(&address) {
            Ok(s) => break s,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("server did not listen: {e}"),
        }
    };
    let mut body = TcpStream::connect(&address).unwrap();
    header
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: local\r\nX-Never-Finished:")
        .unwrap();
    body.write_all(b"POST /api/v1/targets/room/lease HTTP/1.1\r\nHost: local\r\nContent-Type: application/json\r\nContent-Length: 9999\r\n\r\n{").unwrap();
    let start = Instant::now();
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let exit = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "whole-server shutdown exceeded explicit deadline with partial clients"
        );
        thread::sleep(Duration::from_millis(20));
    };
    assert!(exit.success(), "SIGTERM exit was not successful: {exit}");
    assert!(start.elapsed() < Duration::from_secs(5));
}
