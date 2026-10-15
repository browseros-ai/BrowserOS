//! Frozen TS compaction results cover each fallback and cross-window call/approval pairing.
use browseros_agent::{compaction::*, messages::ModelMessage};
use serde_json::Value;

fn fixtures() -> Result<Value, serde_json::Error> {
    serde_json::from_str(include_str!("../src/compaction/golden.json"))
}

#[test]
fn budgets_match_ts_including_invalid_windows() -> Result<(), Box<dyn std::error::Error>> {
    for case in fixtures()?["budgets"].as_array().ok_or("budgets")? {
        let input = match case["input"].as_str() {
            Some("NaN") => Some(f64::NAN),
            Some("Infinity") => Some(f64::INFINITY),
            _ => case["input"].as_f64(),
        };
        let budget = Budget::for_context_window(input);
        assert_eq!(
            budget.window,
            case["expected"]["contextWindow"].as_u64().ok_or("window")?
        );
        assert_eq!(
            budget.threshold,
            case["expected"]["threshold"].as_u64().ok_or("threshold")?
        );
        assert_eq!(
            budget.overhead,
            case["expected"]["overhead"].as_u64().ok_or("overhead")?
        );
        assert_eq!(budget.reserve, budget.window - budget.threshold);
    }
    Ok(())
}

#[test]
fn estimates_match_utf16_and_all_ts_part_kinds() -> Result<(), Box<dyn std::error::Error>> {
    for case in fixtures()?["estimates"].as_array().ok_or("estimates")? {
        let messages: Vec<ModelMessage> = serde_json::from_value(case["messages"].clone())?;
        assert_eq!(
            estimate_tokens(&messages),
            case["expected"].as_u64().ok_or("tokens")?
        );
    }
    Ok(())
}

#[test]
fn prepare_step_matches_ts_and_preserves_pairs() -> Result<(), Box<dyn std::error::Error>> {
    let mut stages = std::collections::BTreeSet::new();
    for case in fixtures()?["prepare"].as_array().ok_or("prepare")? {
        let messages = serde_json::from_value(case["messages"].clone())?;
        let steps = case["steps"]
            .as_array()
            .ok_or("steps")?
            .iter()
            .map(|step| StepUsage {
                input_tokens: step["usage"]["inputTokens"].as_u64(),
                output_tokens: step["usage"]["outputTokens"].as_u64(),
            })
            .collect::<Vec<_>>();
        let (actual, stage) = prepare_step(
            messages,
            &steps,
            &Budget::for_context_window(case["window"].as_f64()),
        );
        assert_eq!(
            serde_json::to_value(actual)?,
            case["expected"],
            "{}",
            case["name"]
        );
        let name = match stage {
            CompactionStage::Untouched => "Untouched",
            CompactionStage::PrunedOld => "PrunedOld",
            CompactionStage::PrunedAll => "PrunedAll",
            CompactionStage::DroppedOldest { still_over_budget } => {
                assert_eq!(Value::Bool(still_over_budget), case["stillOverBudget"]);
                "DroppedOldest"
            }
        };
        assert_eq!(
            Value::String(name.to_owned()),
            case["stage"],
            "{}",
            case["name"]
        );
        stages.insert(name);
    }
    assert_eq!(
        stages.len(),
        4,
        "fixtures must exercise all compaction stages"
    );
    Ok(())
}
