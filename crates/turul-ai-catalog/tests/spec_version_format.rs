mod common;

use common::*;
use turul_ai_catalog::validate::Severity;

// #version-format: components "are non-negative integers"; `01` still denotes one, so valid but noncanonical
#[test]
fn leading_zero_is_spec_version_noncanonical() {
    for v in ["01.0", "1.00", "01.01", "1.007", "001.0"] {
        let r = report(&format!(r#"{{"specVersion":"{v}","entries":[]}}"#));
        assert_only(
            &r,
            "spec-version-noncanonical",
            Severity::Warning,
            "/specVersion",
        );
        assert!(!r.has_errors(), "{v}");
    }
}

// #version-format: every example uses the canonical form; `1.10` is canonical
#[test]
fn canonical_versions_give_no_diagnostic() {
    for v in ["1.0", "1.1", "1.10", "1.100", "1.2"] {
        assert_clean(&report(&format!(r#"{{"specVersion":"{v}","entries":[]}}"#)));
    }
}
