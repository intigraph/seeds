//! Message dedup against the real shared applier, including concurrent windows.
use super::*;
use crate::message::{message_id, window_graph, Message};

fn candidate(at: &str, key: &str) -> (Ctx, Message) {
    (
        Ctx {
            now: at.into(),
            actor: "sender".into(),
            prefix: "test".into(),
            claims: Default::default(),
        },
        Message {
            id: message_id(Some(key), "reader", at, "same message"),
            recipient: "reader".into(),
            sender: Some("sender".into()),
            sent_at: at.into(),
            text: "same message".into(),
            kind: "note".into(),
            delivery_key: Some(key.into()),
            about: vec![],
            read_at: None,
        },
    )
}

#[test]
fn remote_key_binding_is_atomic_across_week_windows() {
    let Some(server) = super::tests::quipu_server() else {
        eprintln!("SKIPPED: set SEEDS_TEST_QUIPU_SERVER for the real atomic message control");
        return;
    };
    let graph = "https://example.org/project/messages";
    let mut b = RemoteBackend::connect(&server.base, graph, None, None, &[]).unwrap();
    let (first_ctx, first) = candidate("2026-11-01T23:59:59Z", "sequential");
    let (retry_ctx, retry) = candidate("2026-11-02T00:00:01Z", "sequential");
    let first_w = window_graph(graph, &first.sent_at).unwrap();
    let retry_w = window_graph(graph, &retry.sent_at).unwrap();
    assert!(
        b.message_send(&first_ctx, &first_w, &first, "test")
            .unwrap()
            .0
    );
    assert!(
        !b.message_send(&retry_ctx, &retry_w, &retry, "test")
            .unwrap()
            .0
    );
    assert_eq!(
        b.message_receipt(&first.id).unwrap().unwrap(),
        (first_w.clone(), first.clone())
    );
    let mut changed = retry.clone();
    changed.recipient = "someone-else".into();
    assert!(b
        .message_send(&retry_ctx, &retry_w, &changed, "test")
        .is_err());
    assert_eq!(
        b.messages(&[first_w.clone(), retry_w.clone()], None)
            .unwrap(),
        vec![first]
    );

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = ["2026-11-01T23:59:59Z", "2026-11-02T00:00:01Z"]
        .into_iter()
        .map(|at| {
            let base = server.base.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut b = RemoteBackend::connect(&base, graph, None, None, &[]).unwrap();
                let (ctx, m) = candidate(at, "concurrent");
                let window = window_graph(graph, at).unwrap();
                barrier.wait();
                b.message_send(&ctx, &window, &m, "race").unwrap().0
            })
        })
        .collect();
    let writes = handles
        .into_iter()
        .map(|h| usize::from(h.join().unwrap()))
        .sum::<usize>();
    assert_eq!(writes, 1);
    let messages = b.messages(&[first_w, retry_w], None).unwrap();
    assert_eq!(messages.len(), 2, "one sequential and one concurrent key");
    assert!(b.snapshot(None).unwrap().seeds.is_empty());
}

#[test]
fn cli_retry_reports_original_window_and_time() {
    use crate::native::{cli::MsgCommand, msg};
    use crate::quipu_backend::QuipuBackend;
    let graph = "https://example.org/project/receipt";
    let mut b = QuipuBackend::in_memory(graph).unwrap();
    let command = MsgCommand::Send {
        to: "reader".into(),
        from: Some("sender".into()),
        kind: "note".into(),
        key: Some("cli-key".into()),
        about: vec![],
        text: vec!["same message".into()],
        message: None,
        file: None,
    };
    let (first, _) = candidate("2026-11-01T23:59:59Z", "cli-key");
    let (retry, _) = candidate("2027-01-04T00:00:01Z", "cli-key");
    let original = msg(true, &first, &mut b, &command).unwrap();
    let receipt = msg(true, &retry, &mut b, &command).unwrap();
    assert!(original.stdout.contains("2026-W44"));
    assert!(receipt.stdout.contains("2026-W44"));
    assert!(receipt.stdout.contains("2026-11-01T23:59:59Z"));
    assert!(!receipt.stdout.contains("2027-01-04"));
}

/// Forward the real applier's committed update, then discard only its response.
#[test]
fn committed_message_with_lost_response_is_not_written_again() {
    use std::io::{BufRead, BufReader, Read, Write};
    let Some(server) = super::tests::quipu_server() else {
        eprintln!("SKIPPED: set SEEDS_TEST_QUIPU_SERVER for the real lost-response control");
        return;
    };
    let graph = "https://example.org/project/lost-message";
    let mut direct = RemoteBackend::connect(&server.base, graph, None, None, &[]).unwrap();
    let (ctx, first) = candidate("2026-11-01T23:59:59Z", "warm");
    let window = window_graph(graph, &ctx.now).unwrap();
    direct.message_send(&ctx, &window, &first, "warm").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy_url = format!("http://{}", listener.local_addr().unwrap());
    let base = server.base.clone();
    let proxy = std::thread::spawn(move || {
        let mut updates = 0;
        for _ in 0..5 {
            let (stream, _) = listener.accept().unwrap();
            let mut r = BufReader::new(stream);
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let words: Vec<_> = line.split_whitespace().collect();
            let (method, path) = (words[0].to_string(), words[1].to_string());
            let mut length = 0;
            let mut content_type = String::new();
            let mut accept = String::new();
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                if h == "\r\n" {
                    break;
                }
                if let Some((k, v)) = h.split_once(':') {
                    match k.to_ascii_lowercase().as_str() {
                        "content-length" => length = v.trim().parse().unwrap(),
                        "content-type" => content_type = v.trim().into(),
                        "accept" => accept = v.trim().into(),
                        _ => (),
                    }
                }
            }
            let mut body = vec![0; length];
            r.read_exact(&mut body).unwrap();
            let request = ureq::request(&method, &format!("{base}{path}"))
                .set("Content-Type", &content_type)
                .set("Accept", &accept);
            let response = if method == "GET" {
                request.call()
            } else {
                request.send_bytes(&body)
            }
            .unwrap();
            let status = response.status();
            let mut body = response.into_string().unwrap();
            let status = if path == "/update" {
                updates += 1;
                body = "lost after commit".into();
                502
            } else {
                status
            };
            write!(
                r.get_mut(),
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        updates
    });
    let mut b = RemoteBackend::connect(&proxy_url, graph, None, None, &[]).unwrap();
    let (_, original) = candidate(&ctx.now, "lost-real");
    let lost = b.message_send(&ctx, &window, &original, "lost");
    assert!(lost.is_ok(), "{lost:?}");
    assert_eq!(proxy.join().unwrap(), 1);
    let (retry_ctx, retry) = candidate("2026-11-02T00:00:01Z", "lost-real");
    let retry_window = window_graph(graph, &retry_ctx.now).unwrap();
    assert!(
        !direct
            .message_send(&retry_ctx, &retry_window, &retry, "retry")
            .unwrap()
            .0
    );
    assert_eq!(
        direct.message_receipt(&retry.id).unwrap().unwrap(),
        (window.clone(), original)
    );
    assert_eq!(
        direct
            .messages(&[window, retry_window], None)
            .unwrap()
            .len(),
        2
    );
}
