//! Redacción mínima del stderr de un adaptador en modo protocolo (HU-019): el adaptador real
//! escribe diagnósticos que pueden incluir el correo de la cuenta u otros datos. Cada palabra
//! (separada por espacios) con forma de correo (`x@y.z`) o que contiene un prefijo de secreto
//! conocido al inicio de palabra (`sk-ant-`, `sk-`, `ghp_`/`gho_`/`ghu_`/`ghs_`/`ghr_`,
//! `github_pat_`, `kbia_`, `xox`, `AKIA`, `-----BEGIN`; una prueba de guarda comprueba el
//! corpus de secretos del núcleo) se sustituye por `[oculto]`. Es una defensa, no un detector exhaustivo: el
//! cuerpo de una clave PEM en las líneas siguientes, por ejemplo, no se reconoce.

/// Sustituto de lo ocultado.
pub const REDACTED: &str = "[oculto]";

const SECRET_PREFIXES: [&str; 12] = [
    "sk-ant-",
    "sk-",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "kbia_",
    "xox",
    "AKIA",
    "-----BEGIN",
];

/// Devuelve la línea con lo sensible sustituido (sin cambios si no hay nada).
pub(crate) fn redact_line(line: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(line);
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let mut changed = false;
    let mut flush = |word: &mut String, out: &mut String| {
        if sensitive(word) {
            out.push_str(REDACTED);
            changed = true;
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in text.chars() {
        if c.is_whitespace() {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    if changed {
        out.into_bytes()
    } else {
        line.to_vec()
    }
}

fn sensitive(word: &str) -> bool {
    email_like(word) || has_secret_prefix(word)
}

/// `algo@dominio.tld`: al menos un carácter antes de `@` y un `.` con algo detrás.
fn email_like(word: &str) -> bool {
    word.char_indices().any(|(at, c)| {
        if c != '@' || at == 0 {
            return false;
        }
        let domain = &word[at + 1..];
        domain
            .find('.')
            .is_some_and(|dot| dot > 0 && domain[dot + 1..].chars().any(char::is_alphanumeric))
    })
}

/// Prefijo conocido al inicio de la palabra o tras un carácter no alfanumérico (así
/// «desk-top» no salta con `sk-`).
fn has_secret_prefix(word: &str) -> bool {
    SECRET_PREFIXES.iter().any(|prefix| {
        word.match_indices(prefix).any(|(at, _)| {
            word[..at]
                .chars()
                .next_back()
                .is_none_or(|before| !before.is_alphanumeric())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redact(text: &str) -> String {
        String::from_utf8(redact_line(text.as_bytes())).unwrap()
    }

    #[test]
    fn hides_emails_and_known_secret_prefixes() {
        assert_eq!(
            redact("cuenta: persona@example.invalid  ok"),
            "cuenta: [oculto]  ok"
        );
        assert_eq!(redact("key=sk-ant-api03-abc fin"), "[oculto] fin");
        assert_eq!(
            redact("ghp_x kbia_1 xoxb-1 AKIAX -----BEGIN"),
            "[oculto] [oculto] [oculto] [oculto] [oculto]"
        );
        assert_eq!(redact("Bearer \"sk-proj-1\""), "Bearer [oculto]");
    }

    #[test]
    fn leaves_normal_text_alone() {
        for text in [
            "[session/create] desk-top task-runner",
            "@scope/paquete versión 1.0",
            "precio 3@ tienda",
            "sin nada",
        ] {
            assert_eq!(redact(text), text);
        }
    }
}
