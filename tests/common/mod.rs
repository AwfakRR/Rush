//! Shared harness: throwaway project dirs and a real `rush` process per test.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Fixture {
    root: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rush-it-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // Canonical so watcher event paths match the server's canonicalized root.
        let root = root.canonicalize().unwrap();
        Self { root }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn write(&self, rel: &str, content: impl AsRef<[u8]>) -> &Self {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
        self
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub fn rush() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rush"))
}

/// A free port never handed out before in this test process, so parallel servers can't collide.
pub fn free_port() -> u16 {
    static USED: Mutex<Vec<u16>> = Mutex::new(Vec::new());
    loop {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut used = USED.lock().unwrap();
        if !used.contains(&port) {
            used.push(port);
            return port;
        }
    }
}

pub struct Server {
    child: Child,
    pub port: u16,
    log: PathBuf,
}

impl Server {
    pub fn start(root: &Path, extra_args: &[&str]) -> Self {
        let port = free_port();
        let log = std::env::temp_dir().join(format!("rush-test-{port}.log"));
        let log_file = std::fs::File::create(&log).unwrap();
        let child = rush()
            .args([
                "--root",
                root.to_str().unwrap(),
                "--port",
                &port.to_string(),
            ])
            .args(extra_args)
            .stdout(Stdio::from(log_file.try_clone().unwrap()))
            .stderr(Stdio::from(log_file))
            .spawn()
            .expect("spawn rush");
        let mut server = Self { child, port, log };
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let deadline = Instant::now() + Duration::from_secs(10);
        // Ready once it answers HTTP. Merely connecting is not enough: dialing a free ephemeral
        // port can self-connect and "succeed" before rush has bound it.
        let listening = || {
            let Ok(mut c) = TcpStream::connect(addr) else {
                return false;
            };
            let mut buf = [0u8; 5];
            c.write_all(b"HEAD /__hmr.js HTTP/1.0\r\n\r\n").is_ok()
                && c.read_exact(&mut buf).is_ok()
                && &buf == b"HTTP/"
        };
        while !listening() {
            assert!(
                Instant::now() < deadline,
                "rush did not start listening on {port}"
            );
            if let Some(status) = server.child.try_wait().unwrap() {
                panic!("rush exited before listening on {port}: {status}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        server
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub fn ws_url(&self) -> String {
        format!("ws://127.0.0.1:{}/__hmr", self.port)
    }

    pub async fn get(&self, path: &str) -> reqwest::Response {
        reqwest::get(self.url(path)).await.unwrap_or_else(|e| {
            panic!(
                "GET {path} failed: {e:?}\nLOG:\n{}",
                std::fs::read_to_string(&self.log).unwrap_or_default()
            )
        })
    }

    pub async fn get_text(&self, path: &str) -> String {
        let resp = self.get(path).await;
        assert!(
            resp.status().is_success(),
            "GET {path} -> {}",
            resp.status()
        );
        resp.text().await.unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.log);
    }
}
