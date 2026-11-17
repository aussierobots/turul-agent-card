//! `DisplayText` contract: every Unicode `Cc` character becomes U+FFFD; all else is kept.
//! #embedded-content-safety: displayed catalog text is untrusted input.

use turul_ai_catalog_client::DisplayText;

/// General Category `Cc`, fixed by the Unicode stability policy.
fn is_cc(c: char) -> bool {
    matches!(c as u32, 0x00..=0x1F | 0x7F..=0x9F)
}

#[test]
fn control_characters_become_replacement_characters() {
    let controls: Vec<char> = (0..=0x10FFFF)
        .filter_map(char::from_u32)
        .filter(|&c| is_cc(c))
        .collect();
    assert_eq!(controls.len(), 65);
    assert!(controls.iter().all(|c| c.is_control()));
    for c in controls {
        let t = DisplayText::new(&c.to_string());
        assert_eq!(t.as_str(), "\u{FFFD}", "U+{:04X}", c as u32);
    }
    let t = DisplayText::new("\u{1b}[2J");
    assert_eq!(t.as_str(), "\u{FFFD}[2J");
    assert_eq!(t.to_string(), "\u{FFFD}[2J");
    assert_eq!(
        DisplayText::new("a\tb\nc\rd\u{7f}e\u{85}f").as_str(),
        "a\u{FFFD}b\u{FFFD}c\u{FFFD}d\u{FFFD}e\u{FFFD}f"
    );
}

// Every scalar value: `Cc` in, U+FFFD out; anything else unchanged, position by position.
#[test]
fn no_output_contains_a_control_character() {
    let input: String = (0..=0x10FFFF).filter_map(char::from_u32).collect();
    let t = DisplayText::new(&input);
    let out = t.as_str();
    assert!(!out.chars().any(is_cc));
    assert!(!out.chars().any(char::is_control));
    assert_eq!(out.chars().count(), input.chars().count());
    for (i, o) in input.chars().zip(out.chars()) {
        let want = if is_cc(i) { '\u{FFFD}' } else { i };
        assert_eq!(o, want, "U+{:04X}", i as u32);
    }
    assert_eq!(t.to_string(), out);
}

// `Cf` and other non-`Cc` characters are text, kept as given.
#[test]
fn other_characters_are_kept() {
    for s in [
        "\u{00A0}",
        "\u{200B}",
        "\u{202E}abc",
        "\u{2028}",
        "\u{FFFD}",
        "e\u{0301}",
        "\u{1F600}",
        "\u{1F469}\u{200D}\u{1F4BB}",
        "Plain ASCII text",
        "日本語",
    ] {
        assert_eq!(DisplayText::new(s).as_str(), s, "{s:?}");
        assert_eq!(DisplayText::new(s).to_string(), s, "{s:?}");
    }
}
