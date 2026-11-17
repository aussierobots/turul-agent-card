//! TUI rendering contract, driven through `ratatui::backend::TestBackend`.
//! Every tree comes from a real `load` of a fixture; states that need a served response are
//! client values pushed onto the loaded tree's public `children` and `warnings`.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::ArtifactSource;
use turul_ai_catalog::resolve;
use turul_ai_catalog_client::{
    Child, DisplayText, FetchPolicy, LoadWarning, Nested, ResolvedCatalog, SkipReason, Source, load,
};
use turul_ai_catalog_tui::{Action, ViewState, render};

const WIDTH: u16 = 220;
const HEIGHT: u16 = 80;

fn load_with(name: &str, policy: &FetchPolicy) -> ResolvedCatalog {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect();
    load(&Source::File(path), policy).expect("fixture loads")
}

fn load_fixture(name: &str) -> ResolvedCatalog {
    load_with(name, &FetchPolicy::default())
}

/// Every buffer cell symbol, row by row.
fn draw(cat: &ResolvedCatalog, state: &ViewState) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    terminal.draw(|f| render(cat, state, f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

fn text(lines: &[String]) -> String {
    lines.join("\n")
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn press(state: &mut ViewState, cat: &ResolvedCatalog, code: KeyCode, n: usize) {
    for _ in 0..n {
        state.on_key(key(code), cat);
    }
}

fn select(cat: &ResolvedCatalog, index: usize) -> ViewState {
    let mut s = ViewState::new();
    press(&mut s, cat, KeyCode::Down, index);
    s
}

fn data_text(cat: &ResolvedCatalog, index: usize) -> String {
    match &cat.document.entries[index].source {
        ArtifactSource::Data(j) => String::from_utf8(j.to_canonical_vec().unwrap()).unwrap(),
        _ => panic!("entry {index} has no data"),
    }
}

fn line_with<'a>(lines: &'a [String], needle: &str) -> &'a str {
    lines
        .iter()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no line contains {needle:?}"))
}

fn no_control_cell(lines: &[String]) {
    for l in lines {
        for c in l.chars() {
            assert!(
                !c.is_control(),
                "control character U+{:04X} in buffer",
                c as u32
            );
        }
    }
}

// #catalog-entry (displayName, "render it as given"), bounded by #embedded-content-safety.
// Each row equals DisplayText of the resolved display name.
#[test]
fn each_row_shows_the_resolved_display_name_as_display_text() {
    let cat = load_fixture("ai-catalog.json");
    let lines = draw(&cat, &ViewState::new());
    assert_eq!(cat.rows.len(), 3);
    for (i, row) in cat.rows.iter().enumerate() {
        let name = resolve::display_name(&cat.document.entries[i], None);
        assert_eq!(row.name, name);
        let shown = DisplayText::new(name);
        line_with(&lines, shown.as_str());
    }
}

// #embedded-content-safety: untrusted input. No control character reaches any cell.
#[test]
fn no_control_character_reaches_the_buffer() {
    let cat = load_fixture("control-characters.json");
    let entry = &cat.document.entries[0];
    let lines = draw(&cat, &ViewState::new());
    no_control_cell(&lines);
    let all = text(&lines);
    assert!(all.contains('\u{FFFD}'));
    let expect = [
        DisplayText::new(entry.display_name.as_deref().unwrap()),
        DisplayText::new(entry.description.as_deref().unwrap()),
        DisplayText::new(entry.identifier.as_str()),
        DisplayText::new("https://example.com/a\u{1b}b"),
        DisplayText::new(&cat.document.host.as_ref().unwrap().display_name),
    ];
    for e in &expect {
        assert!(all.contains(e.as_str()), "missing {:?}", e.as_str());
    }
    // A diagnostic pointer built from an `extensions` key holding ESC.
    assert!(all.contains("\u{FFFD}[2J"));

    let data = load_fixture("control-characters-data.json");
    let lines = draw(&data, &ViewState::new());
    no_control_cell(&lines);
    let shown = DisplayText::new(&data_text(&data, 0));
    line_with(&lines, shown.as_str());
}

// #embedded-content-safety: script-capable content MUST be sandboxed.
#[test]
fn script_capable_data_is_shown_as_inert_text() {
    let cat = load_fixture("script-data.json");
    let lines = draw(&cat, &ViewState::new());
    no_control_cell(&lines);
    for l in &lines {
        assert!(!l.contains('\u{1b}') && !l.contains('\u{7}'));
    }
    let shown = DisplayText::new(&data_text(&cat, 0));
    assert!(shown.as_str().contains("<script>alert(1)</script>"));
    line_with(&lines, shown.as_str());
}

// #embedded-content-safety: "MUST NOT be executed". `type` never selects a renderer.
#[test]
fn data_is_shown_only_as_display_text_of_its_canonical_json() {
    let cat = load_fixture("data-values.json");
    assert_eq!(cat.document.entries.len(), 8);
    for i in 0..4 {
        let expected = DisplayText::new(&data_text(&cat, i));
        assert_eq!(expected.as_str(), data_text(&cat, i + 4));
        for index in [i, i + 4] {
            let lines = draw(&cat, &select(&cat, index));
            line_with(&lines, expected.as_str());
        }
    }
    // The canonical forms themselves.
    assert_eq!(data_text(&cat, 0), r#"{"k":"v1"}"#);
    assert_eq!(data_text(&cat, 1), r#"["a1","b2"]"#);
    assert_eq!(data_text(&cat, 2), r#""s3""#);
    assert_eq!(data_text(&cat, 3), "4242");
}

// #embedded-content-safety; a value the canonical writer refuses is named, not shown.
#[test]
fn data_that_cannot_be_canonicalised_is_named_not_shown() {
    let cat = load_fixture("not-ijson-data.json");
    let ArtifactSource::Data(j) = &cat.document.entries[0].source else {
        panic!("data entry")
    };
    let err = Json::to_canonical_vec(j).unwrap_err();
    let turul_ai_catalog::canonical::CanonicalError::NotIJson { pointer } = err else {
        panic!("NotIJson")
    };
    let lines = draw(&cat, &ViewState::new());
    let want = format!(
        "data not shown: not I-JSON at {}",
        DisplayText::new(&pointer.to_string())
    );
    line_with(&lines, &want);
    let all = text(&lines);
    assert!(!all.contains("visible-secret"));
    assert!(!all.contains('\u{FDD0}'));
}

// #verification-result: "Failure MUST NOT be treated as a verified endorsement".
// Only the count is shown, labelled; no signer, profile or signature content.
#[test]
fn trust_manifests_are_shown_as_not_verified() {
    let cat = load_fixture("ai-catalog.json");
    let label = "trust manifests (not verified)";
    for (index, count) in [(0usize, '1'), (1, '0')] {
        let lines = draw(&cat, &select(&cat, index));
        let l = line_with(&lines, label);
        assert!(l[l.find(label).unwrap() + label.len()..].contains(count));
        let all = text(&lines);
        for forbidden in ["SOC2", "did-web-v1", "detached-jws", "eyJhbGci", "signer"] {
            assert!(!all.contains(forbidden), "{forbidden} shown");
        }
        for l in lines.iter().filter(|l| l.contains("verified")) {
            assert!(l.contains("not verified"), "{l}");
        }
    }
}

fn skipped_line<'a>(lines: &'a [String], reason: &str, url: &str) -> &'a str {
    let url = DisplayText::new(url);
    lines
        .iter()
        .find(|l| l.to_lowercase().contains(reason) && l.contains(url.as_str()))
        .unwrap_or_else(|| panic!("no line with {reason} and {url}"))
}

// #nested-catalog-depth-and-circular-references: a skipped child is shown with reason and url.
#[test]
fn skipped_children_are_shown_with_their_reason() {
    let mut cat = load_fixture("skipped-offline.json");
    let kinds: Vec<_> = cat
        .children
        .iter()
        .map(|n| match &n.child {
            Child::Skipped { url, reason } => (url.clone(), reason.clone()),
            _ => panic!("skipped"),
        })
        .collect();
    assert!(matches!(kinds[0].1, SkipReason::Unresolvable));
    assert!(matches!(kinds[1].1, SkipReason::Fetch(_)));
    cat.children.push(Nested {
        entry: 0,
        child: Child::Skipped {
            url: "https://example.com/deep.json".into(),
            reason: SkipReason::Depth,
        },
    });
    cat.children.push(Nested {
        entry: 1,
        child: Child::Skipped {
            url: "https://example.com/again.json".into(),
            reason: SkipReason::Cycle,
        },
    });
    let lines = draw(&cat, &ViewState::new());
    skipped_line(&lines, "unresolvable", "relative.json");
    let SkipReason::Fetch(e) = &kinds[1].1 else {
        unreachable!()
    };
    let l = skipped_line(&lines, "fetch", &kinds[1].0);
    let _ = l;
    line_with(&lines, DisplayText::new(&e.to_string()).as_str());
    skipped_line(&lines, "depth", "https://example.com/deep.json");
    skipped_line(&lines, "cycle", "https://example.com/again.json");

    let mut policy = FetchPolicy::default();
    policy.max_requests = 0;
    let b = load_with("budget.json", &policy);
    let Child::Skipped { url, reason } = &b.children[0].child else {
        panic!("skipped")
    };
    assert!(matches!(reason, SkipReason::Budget));
    skipped_line(&draw(&b, &ViewState::new()), "budget", url);
}

// #safe-fetching: every LoadWarning is shown with `warning`, its code and its Display.
#[test]
fn load_warnings_are_shown_with_code_and_message() {
    let mut policy = FetchPolicy::default();
    policy.max_requests = 0;
    let cat = load_with("budget.json", &policy);
    assert_eq!(cat.warnings.len(), 1);
    let msg = cat.warnings[0].to_string();
    assert!(msg.contains('0') && msg.contains('1'));
    let lines = draw(&cat, &ViewState::new());
    let l = lines
        .iter()
        .find(|l| l.contains("request-budget"))
        .expect("request-budget line");
    assert!(l.contains("warning"));
    assert!(l.contains(&DisplayText::new(&msg).to_string()));

    let mut cat = load_fixture("diagnostics.json");
    cat.warnings.push(LoadWarning::MediaTypeGeneric);
    let lines = draw(&cat, &ViewState::new());
    let w = lines
        .iter()
        .position(|l| l.contains("media-type-generic"))
        .expect("media-type-generic line");
    assert!(lines[w].contains("warning"));
    let d = lines
        .iter()
        .position(|l| l.contains("identifier-empty"))
        .expect("diagnostic line");
    assert!(w < d, "warnings precede diagnostics");
}

// #validation / #producer-behavior: diagnostics show severity, code and JSON Pointer, and
// the node's Conformance.
#[test]
fn diagnostics_are_shown_with_severity_code_and_pointer() {
    let cat = load_fixture("diagnostics.json");
    let lines = draw(&cat, &ViewState::new());
    let e = line_with(&lines, "identifier-empty");
    assert!(e.contains("/entries/0/identifier"));
    assert!(e.to_lowercase().contains("error"));
    let w = line_with(&lines, "logo-url-not-data-uri");
    assert!(w.contains("/host/logoUrl"));
    assert!(w.to_lowercase().contains("warning"));
    assert!(text(&lines).to_lowercase().contains("conformance"));

    let ok = load_fixture("ai-catalog.json");
    let all = text(&draw(&ok, &ViewState::new())).to_lowercase();
    assert!(all.contains("conformance") && all.contains("discoverable"));
}

// Keys: Up/Down and k/j move within bounds. The detail pane shows the selected entry.
#[test]
fn selection_moves_with_arrow_and_vi_keys_and_stays_in_bounds() {
    let cat = load_fixture("ai-catalog.json");
    let desc = |i: usize| cat.document.entries[i].description.clone().unwrap();
    let shows = |s: &ViewState, i: usize| text(&draw(&cat, s)).contains(&desc(i));
    let only = |s: &ViewState, i: usize| (0..3).all(|j| shows(s, j) == (j == i));

    let mut s = ViewState::new();
    assert!(only(&s, 0));
    press(&mut s, &cat, KeyCode::Up, 3);
    assert!(only(&s, 0), "Up at the first entry stays");
    press(&mut s, &cat, KeyCode::Char('k'), 3);
    assert!(only(&s, 0), "k at the first entry stays");
    assert!(matches!(
        s.on_key(key(KeyCode::Down), &cat),
        Action::Continue
    ));
    assert!(only(&s, 1));
    press(&mut s, &cat, KeyCode::Char('j'), 1);
    assert!(only(&s, 2));
    press(&mut s, &cat, KeyCode::Down, 10);
    assert!(only(&s, 2), "Down at the last entry stays");
    press(&mut s, &cat, KeyCode::Char('j'), 10);
    assert!(only(&s, 2), "j at the last entry stays");
    press(&mut s, &cat, KeyCode::Char('k'), 1);
    assert!(only(&s, 1));
    press(&mut s, &cat, KeyCode::Up, 1);
    assert!(only(&s, 0));

    // Any other key gives Continue and changes nothing.
    let before = draw(&cat, &s);
    for code in [
        KeyCode::Enter,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Tab,
        KeyCode::Char('x'),
        KeyCode::Char('Q'),
        KeyCode::F(1),
    ] {
        assert!(
            matches!(s.on_key(key(code), &cat), Action::Continue),
            "{code:?}"
        );
    }
    assert!(matches!(
        s.on_key(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            &cat
        ),
        Action::Continue
    ));
    assert_eq!(draw(&cat, &s), before);
}

#[test]
fn quit_keys_end_the_session() {
    let cat = load_fixture("ai-catalog.json");
    let mut s = ViewState::new();
    assert!(matches!(
        s.on_key(key(KeyCode::Char('q')), &cat),
        Action::Quit
    ));
    assert!(matches!(s.on_key(key(KeyCode::Esc), &cat), Action::Quit));
    assert!(matches!(
        s.on_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &cat
        ),
        Action::Quit
    ));
}
