use serde_json::json;
use squaremap_compare::report::migration_report_is_complete;

#[test]
fn missing_required_gate_is_not_complete() {
    let report = json!({
        "verdict": "complete",
        "verified": {
            "workspace": {"passed": true},
            "gradle": {"passed": true}
        },
        "blocked": {}
    });

    assert!(!migration_report_is_complete(
        &report,
        &["workspace", "gradle", "live_shadow"]
    ));
}

#[test]
fn blocked_gate_is_not_complete() {
    let report = json!({
        "verdict": "complete",
        "verified": {
            "workspace": {"passed": true},
            "gradle": {"passed": true},
            "live_shadow": {"passed": true}
        },
        "blocked": {
            "fault_restart": "not independently verified"
        }
    });

    assert!(!migration_report_is_complete(
        &report,
        &["workspace", "gradle", "live_shadow"]
    ));
}

#[test]
fn all_required_gates_are_complete() {
    let report = json!({
        "verdict": "complete",
        "verified": {
            "workspace": {"passed": true},
            "gradle": {"passed": true},
            "live_shadow": {"passed": true}
        },
        "blocked": {}
    });

    assert!(migration_report_is_complete(
        &report,
        &["workspace", "gradle", "live_shadow"]
    ));
}

#[test]
fn malformed_blocked_section_is_not_complete() {
    let report = json!({
        "verdict": "complete",
        "verified": {
            "workspace": {"passed": true}
        },
        "blocked": "not an object"
    });

    assert!(!migration_report_is_complete(&report, &["workspace"]));
}
