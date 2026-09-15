//! Wire-level regression tests: connection identity comes from the server,
//! rather than inspecting reqwest's private pool or relying on timing.
use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread::{self, JoinHandle},
    time::Instant,
};

struct KeepAliveServer {
    url: String,
    requests: mpsc::Receiver<(String, usize)>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl KeepAliveServer {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (tx, requests) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut workers = Vec::new();
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let id = workers.len() + 1;
                        let tx = tx.clone();
                        let stopped = stopped.clone();
                        workers.push(thread::spawn(move || serve(stream, id, tx, stopped)));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(e) => panic!("accept failed: {e}"),
                }
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    fn start(
        &self,
        pool: &mut HttpClientPool,
        settings: AppSettings,
        path: &str,
    ) -> InFlightRequest {
        pool.for_settings(settings).start_send(
            HttpMethod::GET,
            format!("{}{path}", self.url),
            vec![],
            None,
            BodyType::None,
        )
    }

    fn received(&self) -> (String, usize) {
        self.requests.recv_timeout(Duration::from_secs(3)).unwrap()
    }
}

impl Drop for KeepAliveServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn serve(
    mut stream: TcpStream,
    id: usize,
    tx: mpsc::Sender<(String, usize)>,
    stop: Arc<AtomicBool>,
) {
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut request = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        let mut chunk = [0; 4096];
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => request.extend_from_slice(&chunk[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(_) => break,
        }
        if !request.windows(4).any(|w| w == b"\r\n\r\n") {
            continue;
        }
        let head = String::from_utf8(std::mem::take(&mut request)).unwrap();
        let path = head.split_whitespace().nth(1).unwrap().to_string();
        if tx.send((path.clone(), id)).is_err() {
            break;
        }
        if path == "/slow" {
            thread::sleep(Duration::from_millis(300));
        }
        let body = if path == "/large" {
            vec![b'x'; 2048]
        } else {
            b"ok".to_vec()
        };
        let close = head.to_ascii_lowercase().contains("connection: close");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nX-Connection-Id: {id}\r\nConnection: {}\r\n\r\n",
            body.len(),
            if close { "close" } else { "keep-alive" },
        );
        // Write headers and body together to avoid delayed-ACK noise in timings.
        if stream
            .write_all(&[response.as_bytes(), &body].concat())
            .is_err()
            || close
        {
            break;
        }
    }
}

fn wait(request: InFlightRequest) -> Result<HttpResponse> {
    runtime().block_on(request.wait())
}

#[test]
fn consecutive_sends_and_download_share_one_connection() {
    let server = KeepAliveServer::new();
    let mut pool = HttpClientPool::default();
    let mut elapsed = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        assert_eq!(
            wait(server.start(&mut pool, AppSettings::default(), "/"))
                .unwrap()
                .body,
            b"ok"
        );
        elapsed.push(start.elapsed());
    }
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("response.bin");
    let download = pool.for_settings(AppSettings::default()).start_download(
        HttpMethod::GET,
        format!("{}/", server.url),
        vec![],
        None,
        BodyType::None,
        destination.clone(),
    );
    assert_eq!(wait(download).unwrap().downloaded_bytes, Some(2));
    assert_eq!(std::fs::read(destination).unwrap(), b"ok");
    let ids: Vec<_> = (0..4).map(|_| server.received().1).collect();
    eprintln!(
        "HTTP/1.1 loopback: first={:?}, subsequent={:?}, connections={ids:?}",
        elapsed[0],
        &elapsed[1..]
    );
    assert_eq!(ids, vec![1, 1, 1, 1]);
}

#[test]
fn request_limits_change_without_replacing_connection_pool() {
    let server = KeepAliveServer::new();
    let mut pool = HttpClientPool::default();
    let mut settings = AppSettings::default();
    wait(server.start(&mut pool, settings.clone(), "/")).unwrap();
    let original = server.received().1;
    settings.total_timeout_ms = 5_000;
    settings.max_response_size_bytes = 1024;
    let error = wait(server.start(&mut pool, settings.clone(), "/large")).unwrap_err();
    assert!(error.downcast_ref::<ResponseLimitExceeded>().is_some());
    assert_eq!(server.received().1, original);
    settings.max_response_size_bytes = 4096;
    assert_eq!(
        wait(server.start(&mut pool, settings, "/large"))
            .unwrap()
            .body
            .len(),
        2048
    );
}

#[test]
fn normalized_transport_settings_reuse_existing_connection() {
    let server = KeepAliveServer::new();
    let mut pool = HttpClientPool::default();
    wait(server.start(&mut pool, AppSettings::default(), "/")).unwrap();
    let settings = AppSettings {
        connect_timeout_ms: 0,
        read_timeout_ms: 0,
        ..AppSettings::default()
    };
    wait(server.start(&mut pool, settings, "/")).unwrap();
    assert_eq!(server.received().1, server.received().1);
}

#[test]
fn changing_connect_timeout_replaces_connection_pool() {
    let server = KeepAliveServer::new();
    let mut pool = HttpClientPool::default();
    wait(server.start(&mut pool, AppSettings::default(), "/")).unwrap();
    let settings = AppSettings {
        connect_timeout_ms: 1234,
        ..AppSettings::default()
    };
    wait(server.start(&mut pool, settings, "/")).unwrap();
    assert_ne!(server.received().1, server.received().1);
}

#[test]
fn changing_read_timeout_preserves_inflight_request_and_applies_to_new_requests() {
    let server = KeepAliveServer::new();
    let mut pool = HttpClientPool::default();
    let old = server.start(&mut pool, AppSettings::default(), "/slow");
    let old_id = server.received().1;
    let settings = AppSettings {
        read_timeout_ms: 50,
        ..AppSettings::default()
    };
    let new = server.start(&mut pool, settings, "/slow");
    assert_ne!(server.received().1, old_id);
    assert!(
        wait(new)
            .unwrap_err()
            .downcast_ref::<RequestTimedOut>()
            .is_some()
    );
    assert_eq!(wait(old).unwrap().body, b"ok");
}

#[test]
fn total_timeout_is_a_per_request_snapshot() {
    let server = KeepAliveServer::new();
    let mut pool = HttpClientPool::default();
    // A short total timeout must not become a sticky Client-level timeout.
    let short = AppSettings {
        total_timeout_ms: 100,
        ..AppSettings::default()
    };
    wait(server.start(&mut pool, short, "/")).unwrap();
    server.received();
    let old = server.start(&mut pool, AppSettings::default(), "/slow");
    server.received();
    let settings = AppSettings {
        total_timeout_ms: 50,
        ..AppSettings::default()
    };
    let new = server.start(&mut pool, settings, "/slow");
    server.received();
    assert!(
        wait(new)
            .unwrap_err()
            .downcast_ref::<RequestTimedOut>()
            .is_some()
    );
    assert_eq!(wait(old).unwrap().body, b"ok");
}

#[test]
fn connection_reuse_can_be_disabled_and_enabled_again() {
    let server = KeepAliveServer::new();
    let mut pool = HttpClientPool::default();
    wait(server.start(&mut pool, AppSettings::default(), "/")).unwrap();
    let original = server.received().1;
    let mut json = serde_json::to_value(AppSettings::default()).unwrap();
    json["connection_reuse"] = false.into();
    let settings: AppSettings = serde_json::from_value(json).unwrap();
    wait(server.start(&mut pool, settings.clone(), "/")).unwrap();
    let disabled_first = server.received().1;
    wait(server.start(&mut pool, settings, "/")).unwrap();
    let disabled_second = server.received().1;
    assert_ne!(original, disabled_first);
    assert_ne!(disabled_first, disabled_second);
    wait(server.start(&mut pool, AppSettings::default(), "/")).unwrap();
    let enabled = server.received().1;
    assert_ne!(
        enabled, original,
        "disabled mode must retire the old cached pool"
    );
    wait(server.start(&mut pool, AppSettings::default(), "/")).unwrap();
    assert_eq!(enabled, server.received().1);
}

#[test]
fn legacy_settings_enable_reuse_and_preserve_existing_values() {
    let settings: AppSettings = serde_json::from_str(
        r#"{
        "connect_timeout_ms": 1234,
        "read_timeout_ms": 2345,
        "total_timeout_ms": 3456,
        "max_response_size_bytes": 4096
    }"#,
    )
    .unwrap();
    assert_eq!(settings.connect_timeout_ms, 1234);
    assert_eq!(settings.read_timeout_ms, 2345);
    assert_eq!(settings.total_timeout_ms, 3456);
    assert_eq!(settings.max_response_size_bytes, 4096);
    assert_eq!(
        serde_json::to_value(settings).unwrap()["connection_reuse"],
        true
    );
}

#[test]
fn explicit_connection_close_prevents_subsequent_reuse() {
    let server = KeepAliveServer::new();
    let mut pool = HttpClientPool::default();
    let request = pool.for_settings(AppSettings::default()).start_send(
        HttpMethod::GET,
        format!("{}/", server.url),
        vec![("Connection".into(), "close".into())],
        None,
        BodyType::None,
    );
    wait(request).unwrap();
    wait(server.start(&mut pool, AppSettings::default(), "/")).unwrap();
    assert_ne!(server.received().1, server.received().1);
}
