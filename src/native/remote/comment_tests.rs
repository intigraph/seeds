//! Actual-client/real-server comment receipts, including a lost HTTP reply.
use super::*;
use crate::engine::{self, CreateReq};
use crate::model::Comment;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{atomic::AtomicUsize, Arc};

fn ctx(actor: &str) -> Ctx {
    Ctx {
        now: "2026-10-01T00:00:00Z".into(),
        actor: actor.into(),
        prefix: "test".into(),
        claims: Default::default(),
    }
}

fn connect(base: &str) -> RemoteBackend {
    RemoteBackend::connect(base, "urn:test:comment-receipts", None, None, &[]).unwrap()
}

fn seed(b: &mut RemoteBackend) -> String {
    engine::create(
        b,
        &ctx("creator"),
        &CreateReq {
            title: "comment receipt controls".into(),
            ..Default::default()
        },
    )
    .unwrap()
    .0
    .id
}

fn batch(id: &str) -> WriteBatch {
    WriteBatch {
        comments: vec![Comment {
            seed: id.into(),
            index: 1,
            author: "declared-author".into(),
            text: "identical fields".into(),
            created_at: ctx("writer").now,
            extra: Default::default(),
        }],
        source: "seeds:comment".into(),
        ..Default::default()
    }
}

/// Forward every request to the real server. Only the first update reply is
/// lost (or stripped for an older server); an optional rival SDK write lands
/// immediately before it. Neither the intended query nor its identity changes.
struct Proxy {
    base: String,
    updates: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[derive(Clone, Copy)]
enum Reply {
    Normal,
    Empty,
    Lost,
    LostAndReadFails,
}

impl Proxy {
    fn new(base: &str, rival: Option<WriteBatch>, reply: Reply) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let proxy_base = format!("http://{}", listener.local_addr().unwrap());
        let upstream = base.to_string();
        let updates = Arc::new(AtomicUsize::new(0));
        let seen = updates.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = std::thread::spawn(move || {
            let mut lost = false;
            while !done.load(Ordering::SeqCst) {
                let Ok((stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut parts = first.split_whitespace();
                let method = parts.next().unwrap();
                let path = parts.next().unwrap();
                let mut length = 0;
                let mut headers = Vec::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    let (key, value) = line.trim().split_once(':').unwrap();
                    if key.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap();
                    }
                    headers.push((key.to_string(), value.trim().to_string()));
                }
                let mut data = vec![0; length];
                reader.read_exact(&mut data).unwrap();
                if lost && path == "/query" && matches!(reply, Reply::LostAndReadFails) {
                    let _ = write!(
                        reader.get_mut(),
                        "HTTP/1.1 500 Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    continue;
                }
                let intercept = path == "/update" && seen.fetch_add(1, Ordering::SeqCst) == 0;
                if intercept {
                    if let Some(batch) = &rival {
                        connect(&upstream).commit(batch, &ctx("rival")).unwrap();
                    }
                }
                let mut request = ureq::request(method, &format!("{upstream}{path}"))
                    .timeout(Duration::from_secs(15));
                for (key, value) in &headers {
                    if !["host", "content-length", "connection"]
                        .contains(&key.to_ascii_lowercase().as_str())
                    {
                        request = request.set(key, value);
                    }
                }
                let response = match request.send_bytes(&data) {
                    Ok(response) | Err(ureq::Error::Status(_, response)) => response,
                    Err(error) => panic!("fixture forwarding failed: {error}"),
                };
                let status = response.status();
                let mut body = Vec::new();
                response.into_reader().read_to_end(&mut body).unwrap();
                if intercept && matches!(reply, Reply::Lost | Reply::LostAndReadFails) {
                    lost = true;
                    continue;
                }
                if intercept && matches!(reply, Reply::Empty) {
                    body.clear();
                }
                let _ = write!(
                    reader.get_mut(),
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = reader.get_mut().write_all(&body);
            }
        });
        Self {
            base: proxy_base,
            updates,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn exact_comment_replay_is_a_noop_but_a_foreign_writer_is_a_conflict() {
    let Some(server) = super::tests::quipu_server() else {
        return;
    };
    let mut b = connect(&server.base);
    let id = seed(&mut b);
    let batch = batch(&id);
    b.commit(&batch, &ctx("writer")).unwrap();
    assert_eq!(b.commit(&batch, &ctx("writer")).unwrap(), 0);
    let error = b.commit(&batch, &ctx("rival")).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Conflict);
    assert_eq!(engine::comment_list(&b, &id, None).unwrap().len(), 1);
}

#[test]
fn lost_reply_confirms_only_the_intended_writer_without_replaying() {
    let Some(server) = super::tests::quipu_server() else {
        return;
    };
    for foreign in [false, true] {
        let mut direct = connect(&server.base);
        let id = seed(&mut direct);
        let proxy = Proxy::new(&server.base, foreign.then(|| batch(&id)), Reply::Lost);
        let mut b = connect(&proxy.base);
        let result = engine::comment_add(
            &mut b,
            &ctx("writer"),
            &id,
            "identical fields",
            Some("declared-author"),
        );
        if foreign {
            let error = result.unwrap_err();
            assert_eq!(error.kind, ErrorKind::Indeterminate);
            assert!(error.message.contains("UNKNOWN"));
        } else {
            assert_eq!(result.unwrap().0.index, 1);
        }
        assert_eq!(
            proxy.updates.load(Ordering::SeqCst),
            1,
            "UNKNOWN never retries"
        );
        assert_eq!(engine::comment_list(&direct, &id, None).unwrap().len(), 1);
    }
}

#[test]
fn legacy_empty_reply_does_not_confuse_a_foreign_comment_with_our_write() {
    let Some(server) = super::tests::quipu_server() else {
        return;
    };
    let mut direct = connect(&server.base);
    let id = seed(&mut direct);
    let proxy = Proxy::new(&server.base, Some(batch(&id)), Reply::Empty);
    let mut b = connect(&proxy.base);
    let (comment, _) = engine::comment_add(
        &mut b,
        &ctx("writer"),
        &id,
        "identical fields",
        Some("declared-author"),
    )
    .unwrap();
    assert_eq!(comment.index, 2, "the rival retained the first index");
    assert_eq!(proxy.updates.load(Ordering::SeqCst), 2);
    assert_eq!(engine::comment_list(&direct, &id, None).unwrap().len(), 2);
}

#[test]
fn distinct_comments_survive_a_real_zero_change_report() {
    let Some(server) = super::tests::quipu_server() else {
        return;
    };
    let mut direct = connect(&server.base);
    let id = seed(&mut direct);
    let mut rival = batch(&id);
    rival.comments[0].text = "a different comment".into();
    let proxy = Proxy::new(&server.base, Some(rival), Reply::Normal);
    let mut b = connect(&proxy.base);
    let (comment, _) = engine::comment_add(
        &mut b,
        &ctx("writer"),
        &id,
        "identical fields",
        Some("declared-author"),
    )
    .unwrap();
    assert_eq!(comment.index, 2);
    assert_eq!(proxy.updates.load(Ordering::SeqCst), 2);
    let comments = engine::comment_list(&direct, &id, None).unwrap();
    assert_eq!(comments.len(), 2);
    assert!(comments.iter().any(|c| c.text == "a different comment"));
    assert!(comments.iter().any(|c| c.text == "identical fields"));
}

#[test]
fn a_failed_readback_after_a_lost_reply_is_unknown_without_retry() {
    let Some(server) = super::tests::quipu_server() else {
        return;
    };
    let mut direct = connect(&server.base);
    let id = seed(&mut direct);
    let proxy = Proxy::new(&server.base, None, Reply::LostAndReadFails);
    let mut b = connect(&proxy.base);
    let error = engine::comment_add(
        &mut b,
        &ctx("writer"),
        &id,
        "identical fields",
        Some("declared-author"),
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Indeterminate);
    assert!(error.message.contains("read-back failed"));
    assert_eq!(proxy.updates.load(Ordering::SeqCst), 1);
    assert_eq!(engine::comment_list(&direct, &id, None).unwrap().len(), 1);
}
