use super::{
    scan::{read_lines, Record},
    Project, UsageEvent,
};
use crate::pricing::TokenBreakdown;
use chrono::DateTime;
use serde::Deserialize;
use serde_json::Value;
use std::{io, path::Path};

fn count(value: &Value, key: &str) -> Option<u64> {
    value.get(key).map_or(Some(0), Value::as_u64)
}
fn breakdown(usage: &Value) -> Option<TokenBreakdown> {
    let speed = usage["speed"].as_str();
    if speed.is_some_and(|speed| !matches!(speed, "standard" | "fast")) {
        return None;
    }
    let (write_5m, write_1h) = if usage["cache_creation"].is_object() {
        (
            count(&usage["cache_creation"], "ephemeral_5m_input_tokens")?,
            count(&usage["cache_creation"], "ephemeral_1h_input_tokens")?,
        )
    } else {
        (count(usage, "cache_creation_input_tokens")?, 0)
    };
    let tokens = TokenBreakdown {
        input: count(usage, "input_tokens")?,
        output: count(usage, "output_tokens")?,
        cache_read: count(usage, "cache_read_input_tokens")?,
        cache_write_5m: write_5m,
        cache_write_1h: write_1h,
        is_fast: speed == Some("fast"),
        is_ultrafast: false,
    };
    tokens
        .total()
        .filter(|total| *total <= 9_007_199_254_740_991)?;
    Some(tokens)
}
// Deserializing selected fields skips prompt/reply bodies even on cwd-only context rows.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Metadata {
    timestamp: Value,
    #[serde(deserialize_with = "present_cwd")]
    cwd: Option<Value>,
    #[serde(rename = "sessionId")]
    session_id: Value,
    #[serde(rename = "requestId")]
    request_id: Value,
    #[serde(rename = "isSidechain")]
    is_sidechain: Value,
    #[serde(rename = "costUSD")]
    cost_usd: Value,
    message: Message,
}
fn present_cwd<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Message {
    id: Value,
    model: Value,
    usage: Value,
}
#[derive(Default)]
struct Parser {
    project: Option<Project>,
    sessions: std::collections::HashMap<String, Option<Project>>,
}
impl Parser {
    fn consume(&mut self, metadata: Metadata) -> Vec<Record> {
        let session = metadata.session_id.as_str();
        if let Some(cwd) = &metadata.cwd {
            let project = cwd.as_str().and_then(Project::from_directory);
            if let Some(session) = session {
                self.sessions.insert(session.into(), project);
            } else {
                self.project = project;
            }
        }
        let project = session
            .map(|session| self.sessions.get(session).cloned().flatten())
            .unwrap_or_else(|| self.project.clone());
        parse_metadata(&metadata, project)
    }
}
pub fn parse(path: &Path) -> io::Result<Vec<Record>> {
    let mut parser = Parser::default();
    let mut result = Vec::new();
    read_lines(path, |_, line| {
        if !line.contains("\"usage\"") && !line.contains("\"cwd\"") {
            return;
        }
        if let Ok(metadata) = serde_json::from_str::<Metadata>(line) {
            result.extend(parser.consume(metadata));
        }
    })?;
    Ok(result)
}
#[cfg(test)]
fn parse_value(value: &Value) -> Vec<Record> {
    serde_json::from_value(value.clone())
        .map(|metadata| Parser::default().consume(metadata))
        .unwrap_or_default()
}
fn parse_metadata(value: &Metadata, project: Option<Project>) -> Vec<Record> {
    let mut result = Vec::new();
    let Some(timestamp) = value
        .timestamp
        .as_str()
        .filter(|text| DateTime::parse_from_rfc3339(text).is_ok())
    else {
        return result;
    };
    let message = &value.message;
    let usage = &message.usage;
    let Some(id) = message.id.as_str().filter(|text| !text.is_empty()) else {
        return result;
    };
    let model = message.model.as_str().unwrap_or("Unknown Model");
    let request = value.request_id.as_str().unwrap_or("");
    let sidechain = value.is_sidechain.as_bool().unwrap_or(false);
    let Some(tokens) =
        breakdown(usage).filter(|tokens| tokens.total().is_some_and(|total| total > 0))
    else {
        return result;
    };
    result.push(Record {
        event: UsageEvent {
            id: format!("{id}:{request}"),
            provider: "claude".into(),
            timestamp: timestamp.into(),
            model: model.into(),
            tokens: tokens.total().unwrap(),
            cost_usd: value
                .cost_usd
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.),
            unpriced_tokens: 0,
            cost_source: None,
            project: project.clone(),
        },
        sidechain,
        message: Some(id.into()),
        breakdown: Some(tokens),
    });
    if let Some(iterations) = usage["iterations"].as_array() {
        for (index, iteration) in iterations.iter().enumerate() {
            if iteration["type"] != "advisor_message" {
                continue;
            }
            let Some(tokens) =
                breakdown(iteration).filter(|tokens| tokens.total().is_some_and(|total| total > 0))
            else {
                continue;
            };
            let advisor_id = format!("{id}:advisor:{index}");
            result.push(Record {
                event: UsageEvent {
                    id: format!("{advisor_id}:{request}"),
                    provider: "claude".into(),
                    timestamp: timestamp.into(),
                    model: iteration["model"].as_str().unwrap_or(model).into(),
                    tokens: tokens.total().unwrap(),
                    cost_usd: None,
                    unpriced_tokens: 0,
                    cost_source: None,
                    project: project.clone(),
                },
                sidechain,
                message: Some(advisor_id),
                breakdown: Some(tokens),
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn includes_cache_tokens_and_rejects_negative_counts() {
        let value = json!({ "timestamp": "2026-09-30T12:00:00Z", "message": { "id": "m", "model": "new-model", "usage": { "input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 20, "cache_creation": { "ephemeral_5m_input_tokens": 3, "ephemeral_1h_input_tokens": 2 } } } });
        assert_eq!(parse_value(&value)[0].event.tokens, 40);
        let mut invalid = value;
        invalid["message"]["usage"]["input_tokens"] = json!(-1);
        assert!(parse_value(&invalid).is_empty());
    }
    #[test]
    fn counts_advisors_but_not_other_iterations_twice() {
        let value = json!({ "timestamp": "2026-09-30T12:00:00Z", "message": { "id": "m", "model": "sonnet", "usage": { "input_tokens": 10, "iterations": [{ "type": "advisor_message", "model": "opus", "input_tokens": 4 }, { "type": "main", "input_tokens": 10 }] } } });
        let result = parse_value(&value);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].event.tokens, 4);
        assert_eq!(result[1].event.model, "opus");
    }
    #[test]
    fn estimates_missing_cost_but_preserves_recorded_cost_and_prices_advisors_separately() {
        let mut value = json!({"timestamp":"2026-09-30T12:00:00Z","costUSD":7.25,
            "message":{"id":"m","model":"claude-opus-5-5","usage":{"input_tokens":1000,
                "output_tokens":100,"cache_creation_input_tokens":9999,
                "cache_creation":{"ephemeral_5m_input_tokens":100,"ephemeral_1h_input_tokens":200},
                "iterations":[{"type":"advisor_message","model":"claude-opus-5-5",
                    "input_tokens":1000,"output_tokens":100}]}}});
        let pricing = crate::pricing::ModelPricing::bundled();
        let records = parse_value(&value);
        assert_eq!(records[0].event.tokens, 1400);
        assert_eq!(records[0].priced(&pricing).cost_usd, Some(7.25));
        assert_eq!(
            records[0].priced(&pricing).cost_source,
            Some(super::super::CostSource::Recorded)
        );
        assert_eq!(records[1].priced(&pricing).cost_usd, Some(0.006));
        value.as_object_mut().unwrap().remove("costUSD");
        assert!((parse_value(&value)[0].priced(&pricing).cost_usd.unwrap() - 0.0081).abs() < 1e-9);
    }
    #[test]
    fn explicit_cwd_attributes_usage_and_advisors_without_retaining_content() {
        let value = json!({"cwd":"/synthetic/work/app", "timestamp":"2026-09-30T12:00:00Z",
            "message":{"id":"m","model":"sonnet","content":"PRIVATE_CLAUDE_REPLY",
                "usage":{"input_tokens":10,"iterations":[{"type":"advisor_message","input_tokens":4}]}}});
        let records = parse_value(&value);
        assert_eq!(records.len(), 2);
        for record in &records {
            let project = record.event.project.as_ref().unwrap();
            assert_eq!(project.directory, "/synthetic/work/app");
            assert_eq!(project.name, "app");
        }
        let stored = serde_json::to_string(&records).unwrap();
        assert!(!stored.contains("PRIVATE_CLAUDE_REPLY"));
        let mut unassigned = value;
        unassigned.as_object_mut().unwrap().remove("cwd");
        let without = parse_value(&unassigned);
        assert_eq!(records[0].event.id, without[0].event.id);
        assert_eq!(records[0].event.tokens, without[0].event.tokens);
        assert!(without[0].event.project.is_none());
    }
    #[test]
    fn cwd_context_is_per_session_and_context_alone_never_manufactures_usage() {
        let mut parser = Parser::default();
        let metadata = |value| serde_json::from_value::<Metadata>(value).unwrap();
        assert!(parser
            .consume(metadata(
                json!({"sessionId":"a", "cwd":"/synthetic/one/app",
            "message":{"role":"user","content":"PRIVATE_CLAUDE_PROMPT"}})
            ))
            .is_empty());
        assert!(parser
            .consume(metadata(
                json!({"sessionId":"b", "cwd":"/synthetic/two/app"})
            ))
            .is_empty());
        let usage = |session| {
            json!({"sessionId":session,"timestamp":"2026-09-30T12:00:00Z",
            "message":{"id":session,"usage":{"input_tokens":10}}})
        };
        let a = parser.consume(metadata(usage("a"))).remove(0);
        let b = parser.consume(metadata(usage("b"))).remove(0);
        assert_ne!(
            a.event.project.as_ref().unwrap().id,
            b.event.project.as_ref().unwrap().id
        );
        assert!(parser.consume(metadata(usage("unknown")))[0]
            .event
            .project
            .is_none());
        parser.consume(metadata(json!({"sessionId":"a","cwd":null})));
        assert!(parser.consume(metadata(usage("a")))[0]
            .event
            .project
            .is_none());
        assert!(!serde_json::to_string(&[a, b])
            .unwrap()
            .contains("PRIVATE_CLAUDE_PROMPT"));
    }
    #[test]
    fn file_paths_and_ordinary_chats_do_not_guess_project_or_usage() {
        let directory = std::env::temp_dir().join(format!(
            "devclash-claude-project-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // The encoded Claude projects folder is deliberately misleading. Only cwd is authoritative.
        let logs = directory.join("-Users-synthetic-guessed-project");
        std::fs::create_dir_all(&logs).unwrap();
        let path = logs.join("synthetic.jsonl");
        let values = [
            json!({"cwd":"/synthetic/real/app","sessionId":"a","message":{"role":"user","content":"PRIVATE_PROMPT"}}),
            json!({"timestamp":"2026-09-30T12:00:00Z","sessionId":"a","message":{"id":"real","usage":{"input_tokens":10}}}),
            json!({"timestamp":"2026-09-30T12:00:00Z","sessionId":"unknown","message":{"id":"no-context","usage":{"input_tokens":7}}}),
            json!({"cwd":"/synthetic/chat","message":{"role":"assistant","content":"ORDINARY_CHAT_NO_USAGE"}}),
        ];
        std::fs::write(&path, values.map(|v| v.to_string()).join("\n")).unwrap();
        let records = parse(&path).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(
            records[0].event.project.as_ref().unwrap().directory,
            "/synthetic/real/app"
        );
        assert!(records[1].event.project.is_none());
        let stored = serde_json::to_string(&records).unwrap();
        assert!(!stored.contains("PRIVATE_PROMPT"));
        assert!(!stored.contains("ORDINARY_CHAT"));
        assert!(!stored.contains("guessed-project"));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
