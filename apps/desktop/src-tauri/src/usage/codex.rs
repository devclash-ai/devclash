use super::{
    scan::{read_lines, Record},
    Project, UsageEvent,
};
use crate::pricing::TokenBreakdown;
use chrono::DateTime;
use serde_json::Value;
use std::{io, path::Path};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Counts {
    input: u64,
    output: u64,
    cached: u64,
    reasoning: u64,
    total: u64,
}
impl Counts {
    fn from(value: &Value) -> Option<Self> {
        if !value.is_object() {
            return None;
        }
        let number = |keys: &[&str]| {
            keys.iter()
                .find_map(|key| {
                    value.get(key).and_then(|value| {
                        value
                            .as_u64()
                            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
                    })
                })
                .unwrap_or(0)
        };
        let input = number(&["input_tokens", "prompt_tokens", "input"]);
        let output = number(&["output_tokens", "completion_tokens", "output"]);
        let cached = number(&[
            "cached_input_tokens",
            "cache_read_input_tokens",
            "cached_tokens",
        ]);
        let reasoning = number(&["reasoning_output_tokens", "reasoning_tokens"]);
        let reported = number(&["total_tokens", "total"]);
        let total = if reported > 0 {
            reported
        } else {
            input.checked_add(output)?.checked_add(reasoning)?
        };
        (total <= 9_007_199_254_740_991).then_some(Self {
            input,
            output,
            cached,
            reasoning,
            total,
        })
    }
    fn subtract(self, previous: Option<Self>) -> Self {
        let old = previous.unwrap_or(Self {
            input: 0,
            output: 0,
            cached: 0,
            reasoning: 0,
            total: 0,
        });
        Self {
            input: self.input.saturating_sub(old.input),
            output: self.output.saturating_sub(old.output),
            cached: self.cached.saturating_sub(old.cached),
            reasoning: self.reasoning.saturating_sub(old.reasoning),
            total: self.total.saturating_sub(old.total),
        }
    }
}
#[derive(Default)]
struct Parser {
    previous: Option<Counts>,
    project: Option<Project>,
    model: Option<String>,
    session: Option<String>,
    meta_seen: bool,
    child: bool,
    created_at: Option<i64>,
    fast: bool,
    ultrafast: bool,
}
impl Parser {
    fn consume(&mut self, value: &Value, file: &str, line: usize) -> Option<Record> {
        let payload = &value["payload"];
        let timestamp = value["timestamp"].as_str().unwrap_or("");
        if value["type"] == "session_meta" && !self.meta_seen {
            self.meta_seen = true;
            self.project = payload["cwd"].as_str().and_then(Project::from_directory);
            self.session = payload["id"].as_str().map(str::to_owned);
            self.child = payload
                .get("forked_from_id")
                .is_some_and(|value| !value.is_null())
                || payload
                    .get("parent_thread_id")
                    .is_some_and(|value| !value.is_null())
                || payload["thread_source"] == "subagent"
                || payload["source"].get("subagent").is_some();
            self.created_at = DateTime::parse_from_rfc3339(timestamp)
                .ok()
                .map(|date| date.timestamp());
            return None;
        }
        if value["type"] == "turn_context" {
            if let Some(cwd) = payload.get("cwd") {
                self.project = cwd.as_str().and_then(Project::from_directory);
            }
            if let Some(model) = model_name(payload) {
                self.model = Some(model.into());
            }
            return None;
        }
        if value["type"] != "event_msg" {
            return None;
        }
        if payload["type"] == "thread_settings_applied" {
            if let Some(tier) = payload["thread_settings"]["service_tier"]
                .as_str()
                .or_else(|| payload["service_tier"].as_str())
            {
                self.fast = matches!(tier, "fast" | "priority");
                self.ultrafast = tier == "ultrafast";
            }
            return None;
        }
        if payload["type"] == "task_started" {
            let starts = payload["started_at"].as_f64();
            let boundary = self.created_at.or_else(|| {
                DateTime::parse_from_rfc3339(timestamp)
                    .ok()
                    .map(|date| date.timestamp())
            });
            if starts
                .zip(boundary)
                .is_some_and(|(starts, boundary)| starts >= boundary as f64)
            {
                self.child = false;
            }
            return None;
        }
        if payload["type"] != "token_count" || DateTime::parse_from_rfc3339(timestamp).is_err() {
            return None;
        }
        let info = &payload["info"];
        let total = Counts::from(&info["total_token_usage"]);
        if self.child {
            if total.is_some() {
                self.previous = total;
            }
            return None;
        }
        if total.is_some() && total == self.previous {
            return None;
        }
        let usage = Counts::from(&info["last_token_usage"])
            .or_else(|| total.map(|counts| counts.subtract(self.previous)));
        if total.is_some() {
            self.previous = total;
        }
        let usage = usage.filter(|counts| counts.total > 0)?;
        if let Some(model) = model_name(payload).or_else(|| model_name(info)) {
            self.model = Some(model.into());
        }
        let session = self.session.as_deref().unwrap_or(file);
        // A copied rollout with the same session ID has the same event ID regardless of its path.
        let id = format!(
            "{session}:{timestamp}:{}",
            total.map(|counts| counts.total).unwrap_or(line as u64)
        );
        Some(
            Record::plain(UsageEvent {
                id,
                provider: "codex".into(),
                timestamp: timestamp.into(),
                model: self.model.as_deref().unwrap_or("Unknown Model").into(),
                tokens: usage.total,
                cost_usd: None,
                unpriced_tokens: 0,
                cost_source: None,
                project: self.project.clone(),
            })
            .with_breakdown(TokenBreakdown {
                input: usage.input.saturating_sub(usage.cached),
                cache_read: usage.cached.min(usage.input),
                output: usage.output,
                is_fast: self.fast,
                is_ultrafast: self.ultrafast,
                ..TokenBreakdown::default()
            }),
        )
    }
}
fn model_name(value: &Value) -> Option<&str> {
    value["model"]
        .as_str()
        .or_else(|| value["model_name"].as_str())
        .or_else(|| value["metadata"]["model"].as_str())
        .filter(|text| !text.trim().is_empty())
}
pub fn parse(path: &Path) -> io::Result<Vec<Record>> {
    let mut parser = Parser::default();
    let mut result = Vec::new();
    let filename = path.file_name().unwrap_or_default().to_string_lossy();
    read_lines(path, |line_number, line| {
        if ![
            "\"turn_context\"",
            "\"token_count\"",
            "\"session_meta\"",
            "\"task_started\"",
            "\"thread_settings_applied\"",
        ]
        .iter()
        .any(|marker| line.contains(marker))
        {
            return;
        }
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            if let Some(record) = parser.consume(&value, &filename, line_number) {
                result.push(record);
            }
        }
    })?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn event(total: u64) -> Value {
        json!({"timestamp":"2026-09-30T12:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":total-10,"cached_input_tokens":20,"output_tokens":10,"total_tokens":total}}}})
    }
    #[test]
    fn cumulative_snapshots_are_deltas_and_cache_is_not_added_twice() {
        let mut parser = Parser::default();
        assert_eq!(
            parser.consume(&event(100), "test", 1).unwrap().event.tokens,
            100
        );
        assert!(parser.consume(&event(100), "test", 2).is_none());
        assert_eq!(
            parser.consume(&event(130), "test", 3).unwrap().event.tokens,
            30
        );
    }
    #[test]
    fn uses_last_usage_when_present() {
        let mut value = event(100);
        value["payload"]["info"]["last_token_usage"] =
            json!({"input_tokens":10,"output_tokens":5,"total_tokens":15});
        assert_eq!(
            Parser::default()
                .consume(&value, "test", 1)
                .unwrap()
                .event
                .tokens,
            15
        );
    }
    #[test]
    fn child_replays_seed_counts_without_charging() {
        let mut parser = Parser::default();
        parser.consume(&json!({"type":"session_meta","timestamp":"2026-09-30T12:00:00Z","payload":{"id":"child","forked_from_id":"parent"}}), "test", 0);
        assert!(parser.consume(&event(100), "test", 1).is_none());
        parser.consume(&json!({"type":"event_msg","timestamp":"2026-09-30T12:00:00Z","payload":{"type":"task_started","started_at":1790769600}}), "test", 2);
        assert_eq!(
            parser.consume(&event(130), "test", 3).unwrap().event.tokens,
            30
        );
    }
    #[test]
    fn pricing_buckets_follow_cache_and_persistent_service_tier_without_extra_reasoning_cost() {
        let mut parser = Parser::default();
        parser.consume(
            &json!({"type":"turn_context","payload":{"model":"gpt-6-astra"}}),
            "test",
            0,
        );
        parser.consume(
            &json!({"type":"event_msg","payload":{"type":"thread_settings_applied",
            "thread_settings":{"service_tier":"ultrafast"}}}),
            "test",
            1,
        );
        let mut value = event(1100);
        value["payload"]["info"]["last_token_usage"] = json!({"input_tokens":1000,
            "cached_input_tokens":900,"output_tokens":100,"reasoning_output_tokens":50,"total_tokens":1100});
        let record = parser.consume(&value, "test", 2).unwrap();
        let breakdown = record.breakdown.unwrap();
        assert_eq!(breakdown.input, 100);
        assert_eq!(breakdown.cache_read, 900);
        assert!(breakdown.is_ultrafast);
        assert_eq!(record.event.tokens, 1100);
        let priced = record.priced(&crate::pricing::ModelPricing::bundled());
        assert!((priced.cost_usd.unwrap() - 0.0414).abs() < 1e-9);
    }

    fn rollout_fixture(
        directory: &Path,
        filename: &str,
        session: &str,
        source: &str,
    ) -> std::path::PathBuf {
        let path = directory.join(filename);
        let records = [
            json!({"timestamp":"2026-09-30T12:00:00Z","type":"session_meta",
                "payload":{"id":session,"source":source,
                    "originator":if source == "vscode" { "Codex Desktop" } else { "codex_cli_rs" }}}),
            json!({"type":"turn_context","payload":{"model":"gpt-6-sol"}}),
            json!({"type":"response_item","payload":{"content":"PRIVATE_CODEX_RESPONSE"}}),
            event(100),
        ];
        std::fs::write(&path, records.map(|value| value.to_string()).join("\n")).unwrap();
        path
    }

    #[test]
    fn desktop_only_and_cli_only_rollouts_need_no_cli_install_or_login_file() {
        let directory = std::env::temp_dir().join(format!(
            "devclash-codex-sources-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        for source in ["vscode", "cli", "exec"] {
            let path = rollout_fixture(&directory, &format!("{source}.jsonl"), source, source);
            let records = parse(&path).unwrap();
            assert_eq!(records.len(), 1, "source: {source}");
            assert_eq!(records[0].event.provider, "codex");
            assert_eq!(records[0].event.model, "gpt-6-sol");
            assert_eq!(records[0].event.tokens, 100);
            assert!(!serde_json::to_string(&records)
                .unwrap()
                .contains("PRIVATE_CODEX_RESPONSE"));
        }
        assert!(!directory.join("auth.json").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn desktop_and_cli_count_independent_sessions_once_across_copied_archives() {
        let directory = std::env::temp_dir().join(format!(
            "devclash-codex-combined-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let desktop = rollout_fixture(&directory, "desktop.jsonl", "desktop-session", "vscode");
        let cli = rollout_fixture(&directory, "cli.jsonl", "cli-session", "cli");
        let archive = directory.join("archived-copy.jsonl");
        std::fs::copy(&desktop, &archive).unwrap();
        let records = [desktop, cli, archive]
            .into_iter()
            .flat_map(|path| parse(&path).unwrap())
            .collect();
        let records = super::super::scan::dedup(records);
        assert_eq!(records.len(), 2);
        assert_eq!(
            records
                .iter()
                .map(|record| record.event.tokens)
                .sum::<u64>(),
            200
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn session_and_latest_turn_cwd_change_only_new_event_attribution() {
        let mut parser = Parser::default();
        parser.consume(&json!({"type":"session_meta","timestamp":"2026-09-30T12:00:00Z",
            "payload":{"id":"session","cwd":"/synthetic/one/app","instructions":"PRIVATE_INSTRUCTIONS"}}), "test", 0);
        let a = parser.consume(&event(100), "test", 1).unwrap();
        assert_eq!(
            a.event.project.as_ref().unwrap().directory,
            "/synthetic/one/app"
        );
        parser.consume(&json!({"type":"turn_context","payload":{"cwd":"/synthetic/two/app","model":"gpt-6-sol"}}), "test", 2);
        // A cwd change does not reset cumulative counts or resurrect a replayed event.
        assert!(parser.consume(&event(100), "test", 3).is_none());
        let b = parser.consume(&event(130), "test", 4).unwrap();
        assert_eq!(b.event.tokens, 30);
        assert_eq!(
            b.event.project.as_ref().unwrap().directory,
            "/synthetic/two/app"
        );
        assert_ne!(
            a.event.project.as_ref().unwrap().id,
            b.event.project.as_ref().unwrap().id
        );
        parser.consume(
            &json!({"type":"turn_context","payload":{"model":"gpt-6-sol"}}),
            "test",
            5,
        );
        let c = parser.consume(&event(160), "test", 6).unwrap();
        assert_eq!(b.event.project, c.event.project);
        parser.consume(
            &json!({"type":"turn_context","payload":{"cwd":"relative/path"}}),
            "test",
            7,
        );
        assert!(parser
            .consume(&event(190), "test", 8)
            .unwrap()
            .event
            .project
            .is_none());
        let stored = serde_json::to_string(&[a, b, c]).unwrap();
        assert!(!stored.contains("PRIVATE_INSTRUCTIONS"));
    }
    #[test]
    fn copied_and_child_rollouts_keep_usage_identity_when_cwd_metadata_is_added() {
        let mut assigned = Parser::default();
        let mut unassigned = Parser::default();
        let session = json!({"type":"session_meta","payload":{"id":"same-session"}});
        unassigned.consume(&session, "first-copy", 0);
        let mut with_cwd = session;
        with_cwd["payload"]["cwd"] = json!("/synthetic/app");
        assigned.consume(&with_cwd, "second-copy", 0);
        let a = assigned.consume(&event(100), "second-copy", 1).unwrap();
        let b = unassigned.consume(&event(100), "first-copy", 1).unwrap();
        assert_eq!(a.event.id, b.event.id);
        assert_eq!(a.event.tokens, b.event.tokens);
        assert!(b.event.project.is_none());
        let mut child = Parser::default();
        child.consume(
            &json!({"type":"session_meta","timestamp":"2026-09-30T12:00:00Z",
            "payload":{"id":"child","forked_from_id":"parent","cwd":"/synthetic/child"}}),
            "child",
            0,
        );
        assert!(child.consume(&event(100), "child", 1).is_none());
        child.consume(
            &json!({"type":"event_msg","timestamp":"2026-09-30T12:00:00Z",
            "payload":{"type":"task_started","started_at":1790769600}}),
            "child",
            2,
        );
        let fresh = child.consume(&event(130), "child", 3).unwrap();
        assert_eq!(fresh.event.tokens, 30);
        assert_eq!(fresh.event.project.unwrap().directory, "/synthetic/child");
    }
}
