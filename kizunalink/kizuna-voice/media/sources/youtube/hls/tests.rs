use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use super::{
    HlsReader,
    fetcher::fetch_segment_into,
    resolver::{PlaylistResolutionError, resolve_playlist},
    types::{ByteRange, Resource},
};

struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    declared_len: Option<usize>,
}

impl Reply {
    fn ok(body: impl AsRef<[u8]>) -> Self {
        let body = body.as_ref().to_vec();
        Self { status: 200, headers: Vec::new(), body, declared_len: None }
    }
    fn range(start: u64, end: u64, total: u64, body: impl Into<Vec<u8>>) -> Self {
        let mut reply = Self::ok(body);
        reply.status = 206;
        reply.headers.push(("Content-Range", format!("bytes {start}-{end}/{total}")));
        reply
    }
}

struct TestServer {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    fn new(handler: impl Fn(&str, &str) -> Reply + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind scripted HTTP server");
        listener.set_nonblocking(true).expect("set nonblocking");
        let url = format!("http://{}", listener.local_addr().expect("server address"));
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !stop_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
                        let mut request = Vec::new();
                        let mut buf = [0; 2048];
                        while !request.windows(4).any(|b| b == b"\r\n\r\n") && request.len() < 8192 {
                            match stream.read(&mut buf) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => request.extend_from_slice(&buf[..n]),
                            }
                        }
                        let request = String::from_utf8_lossy(&request);
                        let path = request.split_whitespace().nth(1).unwrap_or("/");
                        let range = request.lines().find(|line| line.to_ascii_lowercase().starts_with("range:"))
                            .unwrap_or("");
                        let reply = handler(path, range);
                        let mut headers = format!("HTTP/1.1 {} Test\r\nContent-Length: {}\r\nConnection: close\r\n",
                            reply.status, reply.declared_len.unwrap_or(reply.body.len()));
                        for (name, value) in reply.headers {
                            headers.push_str(&format!("{name}: {value}\r\n"));
                        }
                        headers.push_str("\r\n");
                        let _ = stream.write_all(headers.as_bytes());
                        let _ = stream.write_all(&reply.body);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self { url, stop, thread: Some(thread) }
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.url)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("HTTP server thread finished");
        }
    }
}

fn resource(url: String, offset: u64, length: u64) -> Resource {
    Resource { url, range: Some(ByteRange { offset, length }), duration: None }
}

#[tokio::test]
async fn valid_and_ignored_ranges_return_only_requested_bytes() {
    let server = TestServer::new(|path, range| {
        assert_eq!(range.trim().to_ascii_lowercase(), "range: bytes=2-3");
        if path == "/partial" {
            Reply::range(2, 3, 6, b"cd".to_vec())
        } else {
            Reply::ok(b"abcdef".to_vec())
        }
    });
    for path in ["/partial", "/ignored"] {
        let mut out = vec![b'!'];
        fetch_segment_into(&reqwest::Client::new(), &resource(server.url(path), 2, 2), &mut out)
            .await.expect("correct range");
        assert_eq!(out, b"!cd");
    }
}

#[tokio::test]
async fn rejects_bad_ranges_truncation_oversize_and_server_errors_without_committing_bytes() {
    let server = TestServer::new(|path, _| match path {
        "/wrong" => Reply::range(0, 1, 6, b"ab".to_vec()),
        "/malformed" => {
            let mut r = Reply::ok(b"cd".to_vec());
            r.status = 206;
            r.headers.push(("Content-Range", "not-a-range".into()));
            r
        }
        "/truncated" => Reply::range(2, 3, 6, b"c".to_vec()),
        "/oversize" => Reply::range(2, 3, 6, b"cde".to_vec()),
        "/short-full" => Reply::ok(b"abc".to_vec()),
        "/huge-full" => {
            let mut r = Reply::ok(Vec::<u8>::new());
            r.declared_len = Some(super::fetcher::MAX_HLS_SEGMENT_BYTES + 1);
            r
        }
        _ => { let mut r = Reply::ok(Vec::<u8>::new()); r.status = 403; r }
    });
    for path in ["/wrong", "/malformed", "/truncated", "/oversize", "/short-full", "/huge-full", "/forbidden"] {
        let mut out = vec![b'!'];
        assert!(fetch_segment_into(&reqwest::Client::new(), &resource(server.url(path), 2, 2), &mut out).await.is_err(), "{path}");
        assert_eq!(out, b"!", "{path} committed unvalidated data");
    }
}

fn master(child: &str) -> String {
    format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\n{child}\n")
}

#[tokio::test]
async fn playlist_cycles_depth_and_malformed_urls_are_typed() {
    let server = TestServer::new(|path, _| Reply::ok(match path {
        "/self" => master("/self#fragment"),
        "/a" => master("/b"),
        "/b" => master("/a"),
        other if other.starts_with("/level") => {
            let n: usize = other.trim_start_matches("/level").parse().expect("numbered path");
            master(&format!("/level{}", n + 1))
        }
        "/nested" => master("/media"),
        "/bad" => master("http://[invalid"),
        "/media" => "#EXTM3U\n#EXTINF:2.0,\nsegment.aac\n".to_string(),
        _ => master("http://[invalid"),
    }));
    let client = reqwest::Client::new();
    for (path, is_depth) in [("/self", false), ("/a", false), ("/level0", true)] {
        let err = resolve_playlist(&client, &server.url(path)).await.err().expect("must reject cycle or depth");
        if is_depth {
            assert!(err.downcast_ref::<PlaylistResolutionError>().is_some_and(|e| matches!(e, PlaylistResolutionError::Depth)));
        } else {
            assert!(err.downcast_ref::<PlaylistResolutionError>().is_some_and(|e| matches!(e, PlaylistResolutionError::Cycle)));
        }
    }
    for url in ["not a URL".to_string(), server.url("/bad")] {
    let err = resolve_playlist(&client, &url).await.err().expect("malformed URL");
    assert!(err.downcast_ref::<PlaylistResolutionError>().is_some_and(|e| matches!(e, PlaylistResolutionError::InvalidUrl)));
    }
    let (segments, _) = resolve_playlist(&client, &server.url("/nested")).await.expect("valid nesting");
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].url, server.url("/segment.aac"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_second_segment_is_sticky_and_third_segment_is_not_fetched() {
    let third = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&third);
    let server = TestServer::new(move |path, _| match path {
        "/playlist" => Reply::ok("#EXTM3U\n#EXTINF:1,\n/one\n#EXTINF:1,\n/two\n#EXTINF:1,\n/three\n"),
        "/one" => Reply::ok(b"first".to_vec()),
        "/two" => { let mut r = Reply::ok(b"failure".to_vec()); r.status = 500; r }
        _ => { count.fetch_add(1, Ordering::SeqCst); Reply::ok(b"third".to_vec()) }
    });
    let url = server.url("/playlist");
    let result = tokio::time::timeout(Duration::from_secs(8), tokio::task::spawn_blocking(move || {
        let mut reader = HlsReader::new(&url, None, None, None, None).expect("first segment loads");
        let mut buf = [0; 32];
        let n = reader.read(&mut buf).expect("first segment readable");
        assert_eq!(&buf[..n], b"first");
        let first_error = reader.read(&mut buf).expect_err("segment two failed");
        assert!(first_error.to_string().contains("500"), "{first_error}");
        let second_error = reader.read(&mut buf).expect_err("error is sticky, not EOS");
        assert_eq!(first_error.to_string(), second_error.to_string());
    })).await.expect("reader must wake");
    result.expect("reader thread finished");
    assert_eq!(third.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_ts_demux_does_not_forward_raw_bytes() {
    let server = TestServer::new(|path, _| {
        if path == "/playlist" {
            Reply::ok("#EXTM3U\n#EXTINF:1,\n/ts\n")
        } else {
            Reply::ok(vec![0x47; 188])
        }
    });
    let url = server.url("/playlist");
    let result = tokio::task::spawn_blocking(move || HlsReader::new(&url, None, None, None, None)).await.expect("reader thread");
    assert!(result.is_err(), "invalid TS must not be accepted as audio");
}

#[tokio::test]
async fn segmented_and_http_sources_reject_mismatched_ranges() {
    use crate::engine::source::{http::HttpSource, segmented::fetch_chunk};
    let server = TestServer::new(|path, _| match path {
        "/good" => Reply::range(2, 3, 6, b"cd".to_vec()),
        "/wrong" => Reply::range(0, 1, 6, b"ab".to_vec()),
        "/short" => Reply::range(2, 3, 6, b"c".to_vec()),
        _ => Reply::ok(b"abcdef".to_vec()),
    });
    let client = reqwest::Client::new();
    let bytes = fetch_chunk(&client, &server.url("/good"), 2, 2).await.expect("valid chunk");
    assert_eq!(bytes.as_ref(), b"cd");
    for path in ["/wrong", "/short", "/ignored"] {
        assert!(fetch_chunk(&client, &server.url(path), 2, 2).await.is_err(), "{path} segmented");
        assert!(HttpSource::fetch_stream(&client, &server.url(path), 2, Some(2)).await.is_err(), "{path} HTTP");
    }
    let res = HttpSource::fetch_stream(&client, &server.url("/good"), 2, Some(2)).await.expect("valid HTTP response");
    assert_eq!(res.bytes().await.expect("body"), b"cd"[..]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn segmented_probe_requires_valid_one_byte_range() {
    use crate::engine::source::SegmentedSource;
    let server = TestServer::new(|path, _| match path {
        "/wrong" => Reply::range(1, 1, 6, b"b".to_vec()),
        _ => Reply::ok(b"abcdef".to_vec()),
    });
    for path in ["/wrong", "/ignored"] {
        let url = server.url(path);
        let result = tokio::task::spawn_blocking(move || SegmentedSource::new(reqwest::Client::new(), &url))
            .await.expect("probe thread");
        assert!(result.is_err(), "{path}");
    }
}
