// Owned synthetic native reference producer. Not installed or used by sd.
// Like src/native, this separate executable owns filesystem and network I/O;
// the SDK core's disallowed lists remain unchanged and enforced.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use quipu::acquisition::composed::{self, Payload, Request};
use quipu::{store::Datum, Op, Store, Value};
use serde_json::{json, Value as Json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;
const PROJECT: &str = "https://example.org/project/fixture";
const EPHEMERAL: &str = "https://example.org/project/fixture#seeds-ephemeral";
const COLD: &str = "https://example.org/project/excluded";
const TIME: &str = "2026-01-01T00:00:00Z";
fn write_facts(store: &mut Store, body: &Json) {
    let graph = body["graph"].as_str().unwrap();
    let g = store.registered_graph_id(graph).unwrap().unwrap();
    let mut data = vec![];
    for row in body["facts"].as_array().unwrap() {
        let o = &row[2];
        let value = match o["kind"].as_str().unwrap() {
            "iri" => Value::Ref(store.intern(o["value"].as_str().unwrap()).unwrap()),
            "str" => Value::Str(o["value"].as_str().unwrap().into()),
            "int" => Value::Int(o["value"].as_i64().unwrap()),
            "typed" => Value::Typed {
                lexical: o["value"].as_str().unwrap().into(),
                datatype: o["datatype"].as_str().unwrap().into(),
            },
            _ => panic!("unsupported fixture datum"),
        };
        data.push(Datum {
            entity: store.intern(row[0].as_str().unwrap()).unwrap(),
            attribute: store.intern(row[1].as_str().unwrap()).unwrap(),
            value,
            valid_from: TIME.into(),
            valid_to: None,
            op: if body["retract"] == true {
                Op::Retract
            } else {
                Op::Assert
            },
        });
    }
    store.transact_to_graph(&data, TIME, None, None, g).unwrap();
}
fn response(stream: &mut TcpStream, code: u16, body: &str) {
    write!(stream, "HTTP/1.1 {code} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
}
fn main() {
    let owned = std::path::PathBuf::from(std::env::args().nth(1).expect("owned directory"));
    assert!(
        owned.is_dir() && std::fs::read_dir(&owned).unwrap().next().is_none(),
        "fixture directory must already exist and be empty"
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let path = owned.join("main.db");
    let mut store = Store::open(path.to_str().unwrap()).unwrap();
    for graph in [PROJECT, EPHEMERAL, COLD] {
        store.graph_create(graph).unwrap();
    }
    println!("http://{}", listener.local_addr().unwrap());
    std::io::stdout().flush().unwrap();
    for stream in listener.incoming() {
        let mut stream = stream.unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
        let route = first.split_whitespace().nth(1).unwrap();
        let mut len = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                if key.eq_ignore_ascii_case("content-length") {
                    len = value.trim().parse::<usize>().unwrap();
                }
            }
        }
        assert!(len < 64 * 1024, "bounded fixture driver");
        let mut body = vec![0; len];
        reader.read_exact(&mut body).unwrap();
        match route {
            "/health" => response(&mut stream, 200, "{}"),
            "/fixture/stop" => {
                response(&mut stream, 200, "{}");
                break;
            }
            "/fixture/disconnect" => { /* close without a receipt */ }
            "/fixture/timeout" => {
                std::thread::sleep(Duration::from_millis(2200));
            }
            "/fixture/dataset-alias" => {
                store
                    .dataset_create(
                        PROJECT,
                        &[quipu::store::datasets::DatasetMember::new(PROJECT)],
                        TIME,
                        None,
                    )
                    .unwrap();
                response(&mut stream, 200, "{}");
            }
            "/fixture/id" => response(
                &mut stream,
                200,
                &json!({"id":store.store_id().unwrap(),"tx":store.latest_tx_id().unwrap()})
                    .to_string(),
            ),
            "/fixture/facts" => {
                write_facts(&mut store, &serde_json::from_slice(&body).unwrap());
                response(&mut stream, 200, "{}");
            }
            "/fixture/freeze" => {
                store
                    .freeze_graph(COLD, owned.to_str().unwrap(), TIME, None)
                    .unwrap();
                response(&mut stream, 200, "{}");
            }
            "/fixture/verify" => {
                let v: Json = serde_json::from_slice(&body).unwrap();
                let result = (|| -> Result<(), String> {
                    let expected: Request =
                        serde_json::from_value(v["request"].clone()).map_err(|e| e.to_string())?;
                    let payload: Payload = serde_json::from_str(v["wire"].as_str().unwrap())
                        .map_err(|e| e.to_string())?;
                    payload.verify(&expected).map_err(|e| e.to_string())
                })();
                match result {
                    Ok(()) => response(&mut stream, 200, "{}"),
                    Err(e) => response(&mut stream, 422, &json!({"error":e}).to_string()),
                }
            }
            "/fixture/restart" => {
                drop(store);
                store = Store::open(path.to_str().unwrap()).unwrap();
                response(&mut stream, 200, "{}");
            }
            "/acquire/composed" => {
                let request: Request = match serde_json::from_slice(&body) {
                    Ok(request) => request,
                    Err(error) => {
                        response(
                            &mut stream,
                            422,
                            &json!({"error":error.to_string()}).to_string(),
                        );
                        continue;
                    }
                };
                match composed::acquire(&store, &request) {
                    Ok(payload) => response(
                        &mut stream,
                        200,
                        &serde_json::to_string_pretty(&payload).unwrap(),
                    ),
                    Err(error) => response(
                        &mut stream,
                        422,
                        &json!({"error":error.to_string()}).to_string(),
                    ),
                }
            }
            _ => panic!("unregistered synthetic route"),
        }
    }
}
