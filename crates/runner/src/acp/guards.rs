//! Pruebas de guarda (solo con la feature `supervisor`, que enlaza el núcleo): las copias que
//! el runner mantiene porque sin esa feature no enlaza `kanbania-core` no se separan de su
//! original.

use kanbania_core::features::runs::ACP_CAPABILITIES;
use kanbania_core::platform::text::{contains_secret_like, has_invisible_formatting};
use serde_json::json;

use super::client::declared_capabilities;
use super::normalize::is_invisible_formatting;
use crate::redact::{REDACTED, redact_line};

/// (a) El conjunto de formato invisible coincide con el del núcleo para todo `char`.
#[test]
fn invisible_formatting_matches_the_core_for_every_char() {
    let mut buffer = [0u8; 4];
    for c in (0..=u32::from(char::MAX)).filter_map(char::from_u32) {
        assert_eq!(
            is_invisible_formatting(c),
            has_invisible_formatting(c.encode_utf8(&mut buffer)),
            "U+{:04X}",
            u32::from(c)
        );
    }
}

/// (b) Toda capacidad que el cliente puede declarar (más `modes`) la acepta el núcleo.
#[test]
fn declared_capabilities_are_known_to_the_core() {
    let all = json!({
        "loadSession": true,
        "promptCapabilities": {"image": true, "audio": true, "embeddedContext": true},
        "mcpCapabilities": {"http": true, "sse": true}
    });
    let mut names = declared_capabilities(Some(&all));
    assert_eq!(names.len(), 6, "{names:?}");
    names.push("modes");
    for name in names {
        assert!(ACP_CAPABILITIES.contains(&name), "{name}");
    }
}

/// (c) Corpus de secretos de las pruebas de `crates/core/src/platform/text.rs`
/// (`third_party_secrets_are_detected`, copiado): cada palabra que el núcleo considera un
/// secreto sale oculta del stderr del adaptador.
#[test]
fn core_secret_corpus_is_redacted() {
    for secret in [
        "kbia_0123456789abcdef",
        "token ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789",
        "gho_0123456789abcdefghij",
        "github_pat_11ABCDEFG0123456789_abcdefghijklmnop",
        "ANTHROPIC=sk-ant-api03-abcdefghijklmnopqrstuv",
        "sk-proj-abc123def456ghi789jkl012",
        "AKIAIOSFODNN7EXAMPLE",
        "xoxb-1234567890-abcdefghij",
        "-----BEGIN RSA PRIVATE KEY-----\nMIIE…",
        "-----BEGIN OPENSSH PRIVATE KEY-----",
        "-----BEGIN PRIVATE KEY-----",
    ] {
        // stderr llega por líneas.
        for line in secret.lines() {
            let redacted = String::from_utf8(redact_line(line.as_bytes())).unwrap_or_default();
            assert!(!contains_secret_like(&redacted), "{line:?} → {redacted:?}");
            for word in line.split_whitespace().filter(|w| contains_secret_like(w)) {
                assert!(!redacted.contains(word), "{line:?} → {redacted:?}");
            }
        }
        let first = secret.lines().next().unwrap_or_default();
        let redacted = String::from_utf8(redact_line(first.as_bytes())).unwrap_or_default();
        assert!(redacted.contains(REDACTED), "{first:?} → {redacted:?}");
    }
}
