//! The server stops when it is asked to, in the way it is asked.
//!
//! Through the real binary, because what this guards is not in any module: a
//! container runtime stops the server with SIGTERM, and in the image the
//! server is PID 1, where a signal with no handler is simply ignored. Docker
//! then waits ten seconds and kills it - requests in flight cut off, and every
//! deploy ten seconds longer than it needs to be.
#![cfg(unix)]

mod support;

use std::{
    net::{TcpListener, TcpStream},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use support::TestServer;

/// Comfortably under the ten seconds Docker waits before it kills.
const STOPS_WITHIN: Duration = Duration::from_secs(5);

#[tokio::test]
async fn sigterm_stops_the_server_promptly_and_cleanly() {
    let Some(server) = TestServer::start().await else { return };
    let Ok(url) = std::env::var("DATABASE_URL") else { return };
    let database = support::replace_database(&url, &server.database_name());

    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_kasl-server"))
        .env("DATABASE_URL", database)
        .env("KASL_SERVER_ADDR", format!("127.0.0.1:{port}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the binary should start");

    // Listening means started: migrations ran and the signal handler is
    // installed before the listener is.
    let started = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("the server exited before it listened: {status}");
        }
        assert!(started.elapsed() < Duration::from_secs(60), "the server never started listening");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let killed = Command::new("kill").args(["-TERM", &child.id().to_string()]).status().expect("kill should run");
    assert!(killed.success());

    let asked = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if asked.elapsed() > STOPS_WITHIN {
            child.kill().ok();
            panic!("still running {STOPS_WITHIN:?} after SIGTERM: a container runtime would have killed it");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(status.success(), "a requested stop is a clean exit, not {status}");

    server.close().await;
}
