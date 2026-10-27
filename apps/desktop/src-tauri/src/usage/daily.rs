use super::{CostSource, UsageEvent};
use chrono::{DateTime, Local, TimeZone};
use std::collections::HashMap;

pub fn totals(events: Vec<UsageEvent>) -> Vec<UsageEvent> {
    totals_ref(&events)
}

pub fn totals_ref(events: &[UsageEvent]) -> Vec<UsageEvent> {
    let mut groups: HashMap<(String, String, String, Option<String>), UsageEvent> = HashMap::new();
    for event in events {
        let unpriced_tokens = if event.cost_usd.is_none() {
            event.tokens
        } else {
            event.unpriced_tokens
        };
        let Ok(date) = DateTime::parse_from_rfc3339(&event.timestamp) else {
            continue;
        };
        let day = date.with_timezone(&Local).date_naive();
        let key = (
            event.provider.clone(),
            day.to_string(),
            event.model.clone(),
            event.project.as_ref().map(|project| project.id.clone()),
        );
        if let Some(total) = groups.get_mut(&key) {
            total.tokens = total.tokens.saturating_add(event.tokens);
            total.cost_usd = match (total.cost_usd, event.cost_usd) {
                (Some(a), Some(b)) => Some(a + b),
                (some @ Some(_), None) | (None, some @ Some(_)) => some,
                _ => None,
            };
            total.unpriced_tokens = total.unpriced_tokens.saturating_add(unpriced_tokens);
            total.cost_source = match (total.cost_source, event.cost_source) {
                (Some(a), Some(b)) if a != b => Some(CostSource::Mixed),
                (Some(source), _) | (_, Some(source)) => Some(source),
                _ => None,
            };
        } else {
            let Some(noon) = day
                .and_hms_opt(12, 0, 0)
                .and_then(|time| Local.from_local_datetime(&time).earliest())
            else {
                continue;
            };
            let identity = serde_json::to_string(&key).expect("daily identity is serializable");
            groups.insert(
                key,
                UsageEvent {
                    id: format!("daily:{}", crate::cloud::records::hash(&identity)),
                    timestamp: noon.to_rfc3339(),
                    unpriced_tokens,
                    ..event.clone()
                },
            );
        }
    }
    let mut result = groups.into_values().collect::<Vec<_>>();
    result.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then(a.id.cmp(&b.id)));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(id: &str, model: &str, tokens: u64, cost_usd: Option<f64>) -> UsageEvent {
        UsageEvent {
            id: id.into(),
            provider: "codex".into(),
            timestamp: "2026-09-30T12:00:00Z".into(),
            model: model.into(),
            tokens,
            cost_usd,
            unpriced_tokens: 0,
            cost_source: None,
            project: None,
        }
    }
    #[test]
    fn daily_payload_preserves_provider_and_model_totals() {
        let records = (0..10_000)
            .map(|index| {
                event(
                    &index.to_string(),
                    if index % 2 == 0 { "first" } else { "second" },
                    100,
                    Some(0.01),
                )
            })
            .collect();
        let result = totals(records);
        assert_eq!(result.len(), 2);
        assert_eq!(
            result.iter().map(|event| event.tokens).sum::<u64>(),
            1_000_000
        );
    }
    #[test]
    fn partial_pricing_never_turns_into_a_complete_spend_estimate() {
        let result = totals(vec![
            event("a", "first", 100, Some(0.01)),
            event("b", "first", 200, None),
        ]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].tokens, 300);
        assert_eq!(result[0].cost_usd, Some(0.01));
        assert_eq!(result[0].unpriced_tokens, 200);
    }
    #[test]
    fn projects_and_unassigned_keep_distinct_stable_daily_rows_and_reconcile_totals() {
        let mut a = event("a", "first", 100, Some(0.01));
        a.project = super::super::Project::from_directory("/synthetic/one/app");
        a.cost_source = Some(CostSource::Recorded);
        let mut b = event("b", "first", 200, Some(0.02));
        b.project = a.project.clone();
        b.cost_source = Some(CostSource::Estimated);
        let mut c = event("c", "first", 300, None);
        c.project = super::super::Project::from_directory("/synthetic/two/app");
        let input = vec![a, b, c, event("d", "first", 400, None)];
        let result = totals(input.clone());
        assert_eq!(result.len(), 3);
        assert_eq!(result.iter().map(|e| e.tokens).sum::<u64>(), 1000);
        assert_eq!(result.iter().map(|e| e.unpriced_tokens).sum::<u64>(), 700);
        assert!((result.iter().filter_map(|e| e.cost_usd).sum::<f64>() - 0.03).abs() < 1e-9);
        let combined = result
            .iter()
            .find(|e| e.tokens == 300 && e.cost_usd.is_some())
            .unwrap();
        assert_eq!(combined.cost_source, Some(CostSource::Mixed));
        assert_eq!(
            result
                .iter()
                .map(|e| &e.id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3
        );
        let mut reversed = input;
        reversed.reverse();
        let again = totals(reversed);
        assert_eq!(
            result.iter().map(|e| &e.id).collect::<Vec<_>>(),
            again.iter().map(|e| &e.id).collect::<Vec<_>>()
        );
        assert_eq!(
            result.iter().map(|e| e.project.clone()).collect::<Vec<_>>(),
            again.iter().map(|e| e.project.clone()).collect::<Vec<_>>()
        );
        let reload = totals(result.clone());
        assert_eq!(
            result.iter().map(|e| &e.id).collect::<Vec<_>>(),
            reload.iter().map(|e| &e.id).collect::<Vec<_>>()
        );
    }
}
