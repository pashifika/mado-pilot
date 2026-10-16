//! Lists model choices and scoped test results without loading models or reports.

use mado_pilot::{DEFAULT_OCR_PROFILE_ID, OcrEvaluationOutcome, OcrProfileConfig, ocr_profiles};

fn main() -> mado_pilot::Result<()> {
    println!("default={DEFAULT_OCR_PROFILE_ID}");
    for profile in ocr_profiles() {
        // Configuration only: replace these paths before constructing an engine.
        let selected =
            OcrProfileConfig::from_id(profile.profile_id, "unread-model-root", "unread-runtime")?;
        println!(
            "profile={} name={}",
            selected.profile_id(),
            profile.display_name
        );
        for evaluation in profile.evaluations {
            println!(
                "  target={} provider={} criteria={}",
                evaluation.target, evaluation.provider, evaluation.criteria,
            );
            if let Some(evidence) = evaluation.evidence {
                println!(
                    "  report={} measured_source={} evaluator={}",
                    evidence.report, evidence.measured_source_revision, evidence.evaluator_revision,
                );
            } else {
                println!("  evidence=unavailable");
            }
            for rule in evaluation.rules {
                let outcome = match rule.outcome {
                    OcrEvaluationOutcome::Passed => "passed",
                    OcrEvaluationOutcome::Failed => "failed",
                    OcrEvaluationOutcome::NotRun => "not_run",
                    OcrEvaluationOutcome::Incomplete => "incomplete",
                };
                println!("  {}={outcome}", rule.rule_id);
            }
        }
    }
    Ok(())
}
