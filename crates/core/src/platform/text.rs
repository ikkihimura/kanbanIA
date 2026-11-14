//! Comprobaciones de texto comunes a varios slices.

/// Caracteres de formato invisibles (bidi, ancho cero, etiquetas Unicode, guiones blandos…) que
/// podrían disfrazar un texto ante quien lo revisa u ocultar instrucciones a otro agente. Se
/// rechazan en todo texto que escriben los agentes y en resúmenes y motivos.
pub fn has_invisible_formatting(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{180E}'
            | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}'
            | '\u{E0000}'..='\u{E007F}')
    })
}

/// `true` si el texto contiene algo con forma de token de agente de Kanban IA: `kbia_` (sin
/// distinguir mayúsculas) seguido de al menos 8 hexadecimales. El formato lo genera
/// `features::access` (una prueba allí comprueba que sus tokens se detectan).
pub fn contains_token_like(text: &str) -> bool {
    let bytes = text.as_bytes();
    find_all_ci(bytes, b"kbia_").any(|end| run_len(&bytes[end..], |b| b.is_ascii_hexdigit()) >= 8)
}

/// `true` si el texto contiene algo con forma de secreto: un token de Kanban IA o credenciales
/// conocidas de terceros (GitHub, Anthropic, OpenAI, AWS, Slack) o una clave privada PEM. Es
/// una defensa para no congelar ni enviar a agentes secretos pegados por error, no un detector
/// exhaustivo. Los prefijos exigen un límite de palabra delante y un mínimo de caracteres del
/// alfabeto del secreto detrás, para no saltar con texto normal (p. ej. «desk-top»).
pub fn contains_secret_like(text: &str) -> bool {
    let bytes = text.as_bytes();
    let alnum = |b: u8| b.is_ascii_alphanumeric();
    let key_chars = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-');
    contains_token_like(text)
        || ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
            .iter()
            .any(|p| prefixed(bytes, p.as_bytes(), 20, alnum, false))
        || prefixed(bytes, b"github_pat_", 20, |b| alnum(b) || b == b'_', false)
        || prefixed(bytes, b"sk-ant-", 20, key_chars, false)
        // `sk-` genérico (OpenAI y otros): además exige algún dígito.
        || prefixed(bytes, b"sk-", 20, key_chars, true)
        || prefixed(bytes, b"AKIA", 16, |b| b.is_ascii_uppercase() || b.is_ascii_digit(), false)
        || ["xoxa-", "xoxb-", "xoxp-", "xoxr-", "xoxs-"]
            .iter()
            .any(|p| prefixed(bytes, p.as_bytes(), 10, |b| alnum(b) || b == b'-', true))
        || pem_private_key(text)
}

/// Prefijo (sensible a mayúsculas) al inicio de una palabra seguido de al menos `min`
/// caracteres de `allowed` (y de algún dígito si `need_digit`).
fn prefixed(
    bytes: &[u8],
    prefix: &[u8],
    min: usize,
    allowed: impl Fn(u8) -> bool,
    need_digit: bool,
) -> bool {
    bytes
        .windows(prefix.len())
        .enumerate()
        .filter(|(at, window)| *window == prefix && word_start(bytes, *at))
        .any(|(at, _)| {
            let rest = &bytes[at + prefix.len()..];
            let len = run_len(rest, &allowed);
            len >= min && (!need_digit || rest[..len].iter().any(u8::is_ascii_digit))
        })
}

fn word_start(bytes: &[u8], at: usize) -> bool {
    at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_')
}

/// Posiciones justo después de cada aparición de `needle` (sin distinguir mayúsculas ASCII).
fn find_all_ci<'a>(bytes: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    bytes
        .windows(needle.len())
        .enumerate()
        .filter(move |(_, w)| w.eq_ignore_ascii_case(needle))
        .map(move |(at, _)| at + needle.len())
}

fn run_len(bytes: &[u8], allowed: impl Fn(u8) -> bool) -> usize {
    bytes.iter().take_while(|b| allowed(**b)).count()
}

/// Cabecera PEM `-----BEGIN … PRIVATE KEY-----` (RSA, EC, OPENSSH, cifrada…).
fn pem_private_key(text: &str) -> bool {
    text.match_indices("-----BEGIN ").any(|(at, _)| {
        let rest = &text[at..];
        let line = rest.lines().next().unwrap_or(rest);
        line.contains("PRIVATE KEY-----")
    })
}

#[cfg(test)]
mod tests {
    use super::{contains_secret_like, contains_token_like, has_invisible_formatting};

    #[test]
    fn token_like_text_is_detected() {
        assert!(contains_token_like("usa KBIA_0123ABCD aquí"));
        for clean in ["kbia_", "kbia_1234", "kbia_default", "sin nada"] {
            assert!(!contains_token_like(clean), "{clean:?}");
        }
    }

    #[test]
    fn third_party_secrets_are_detected() {
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
            assert!(contains_secret_like(secret), "{secret:?}");
        }
    }

    #[test]
    fn normal_text_is_not_a_secret() {
        for text in [
            "Usa el desk-organization-procedure-guide del equipo",
            "risk-assessment-for-the-new-release-plan",
            "tarea sk-corta",
            "la rama ghp_ no existe",
            "AKIA es un prefijo de AWS",
            "AKIAshort",
            "xoxb-",
            "-----BEGIN CERTIFICATE-----",
            "-----BEGIN PUBLIC KEY-----",
            "Título con acentos, ñ y emoji 🚀",
            "mask-0123456789abcdefghijk (no empieza palabra)",
        ] {
            assert!(!contains_secret_like(text), "{text:?}");
        }
    }

    #[test]
    fn hidden_formatting_is_detected_and_plain_text_is_not() {
        for hidden in [
            "a\u{200B}b",
            "\u{202E}txt",
            "tag\u{E0041}",
            "x\u{2060}",
            "blando\u{00AD}",
            "\u{FEFF}",
        ] {
            assert!(has_invisible_formatting(hidden), "{hidden:?}");
        }
        assert!(!has_invisible_formatting(
            "Título con acentos, ñ y emoji 🚀\n\tbien"
        ));
    }
}
