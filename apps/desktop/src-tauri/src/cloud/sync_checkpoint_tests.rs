//! Real HTTP acknowledgement/checkpoint recovery without provider or account credentials.
use super::*;
use std::{io::{BufRead, BufReader, Read, Write}, net::TcpListener, thread};

fn record(index: usize) -> records::CloudRecord {
    records::CloudRecord {
        id: format!("synthetic-{index}"), provider: "Codex".into(), model: "test-model".into(),
        tokens: 100, occurred_at: "2026-10-08T00:00:00Z".into(), cost_usd: None,
        unpriced_tokens: 100, kind: "event".into(), source_id: None, revision_at: None, project_id: None,
    }
}
fn directory() -> PathBuf {
    std::env::temp_dir().join(format!("devclash-checkpoint-{}-{}", std::process::id(), Utc::now().timestamp_nanos_opt().unwrap()))
}
fn store(path: PathBuf, origin: &str, preferences: Preferences) -> CloudStore {
    CloudStore {
        api: Some(Api::new(origin).unwrap()), app_origin: Ok(origin.into()), path,
        credential: Mutex::new(Some(Credential {token: "synthetic-token".into(), user: User {id: "synthetic-user".into(), email: "qa@example.invalid".into()}, expires_at: (Utc::now()+chrono::Duration::days(1)).to_rfc3339()})),
        preferences: Mutex::new(preferences), status: Mutex::new(Status::default()),
        syncing: Mutex::new(()), browser_login: Mutex::new(None), keyring: None,
    }
}
fn preferences(origin: &str) -> Preferences {
    Preferences {origin: origin.into(), user_id: "synthetic-user".into(), sync_enabled: true, ..Preferences::default()}
}
fn server(replies: Vec<bool>) -> (String, thread::JoinHandle<usize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut seen = HashSet::new();
        for success in replies {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut line = String::new(); reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") { length = value.trim().parse::<usize>().unwrap(); }
                if line == "\r\n" {break;}
            }
            let mut bytes = vec![0; length]; reader.read_exact(&mut bytes).unwrap();
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            let records = request["records"].as_array().unwrap();
            assert!(records.len() <= 900);
            let (status, body) = if success {
                let accepted = records.iter().filter(|row| seen.insert(row["id"].as_str().unwrap().to_owned())).count();
                (200, json!({"accepted":accepted,"duplicates":records.len()-accepted}))
            } else {(503, json!({"error":"Synthetic outage"}))};
            let body = body.to_string();
            write!(stream,"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
        seen.len()
    });
    (origin, handle)
}
#[test]
fn uncheckpointed_success_replays_after_restart_without_loss_or_double_count() {
    let (origin, server) = server(vec![true; 6]);
    let directory = directory(); let path = directory.join("cloud-sync.json");
    let cloud = store(path.clone(), &origin, preferences(&origin));
    let crash_state = Mutex::new(None);
    let callbacks = Mutex::new(Vec::new());
    cloud.sync_automatic_with_progress((0..1801).map(record).collect(), |status| {
        callbacks.lock().unwrap().push(status.pending_count);
        if crash_state.lock().unwrap().is_none() {
            let saved: Preferences = crate::storage::read_json(&path).unwrap();
            assert_eq!(saved.pending.len(),1801);
            assert!(saved.acknowledged.is_empty());
            *crash_state.lock().unwrap() = Some(saved);
        }
    }).unwrap();
    assert_eq!(*callbacks.lock().unwrap(), vec![901,1,0]);
    // The persisted pre-checkpoint state is exactly what a killed process would reload.
    let restarted = store(path.clone(), &origin, crash_state.into_inner().unwrap().unwrap());
    let done = restarted.sync(vec![]).unwrap();
    assert_eq!(done.pending_count,0);
    let saved: Preferences = crate::storage::read_json(&path).unwrap();
    assert_eq!(saved.acknowledged.len(),1801);
    assert_eq!(server.join().unwrap(),1801);
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn failure_checkpoints_successful_batches_and_keeps_unsent_records() {
    let (origin, server) = server(vec![true,false]);
    let directory = directory(); let path = directory.join("cloud-sync.json");
    let cloud = store(path.clone(), &origin, preferences(&origin));
    assert!(cloud.sync((0..1801).map(record).collect()).is_err());
    let saved: Preferences = crate::storage::read_json(&path).unwrap();
    assert_eq!(saved.pending.len(),901);
    assert_eq!(saved.acknowledged.len(),900);
    assert_eq!(saved.retry_attempts,1);
    assert!(saved.next_retry_at.is_some());
    assert_eq!(server.join().unwrap(),900);
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn long_cycle_checkpoints_by_tenth_batch_and_saves_final_tail() {
    let (origin, server) = server(vec![true; 12]);
    let directory = directory(); let path = directory.join("cloud-sync.json");
    let cloud = store(path.clone(), &origin, preferences(&origin));
    cloud.sync_automatic_with_progress((0..9901).map(record).collect(), |status| {
        if status.pending_count == 901 {
            let saved: Preferences = crate::storage::read_json(&path).unwrap();
            assert_eq!(saved.pending.len(),901);
            assert_eq!(saved.acknowledged.len(),9000);
        }
    }).unwrap();
    let saved: Preferences = crate::storage::read_json(&path).unwrap();
    assert!(saved.pending.is_empty()); assert_eq!(saved.acknowledged.len(),9901);
    assert!(saved.last_synced_at.is_some());
    assert_eq!(server.join().unwrap(),9901);
    std::fs::remove_dir_all(directory).unwrap();
}
