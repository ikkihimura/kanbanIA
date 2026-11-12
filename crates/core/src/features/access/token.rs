//! Formato de los tokens de agente: prefijo fijo + 32 bytes aleatorios del sistema en hex.

use sha2::{Digest, Sha256};

use crate::error::CoreError;

const PREFIX: &str = "kbia_";
const RANDOM_BYTES: usize = 32;
pub(super) const TOKEN_LEN: usize = PREFIX.len() + RANDOM_BYTES * 2;
const HINT_LEN: usize = 4;

pub(super) struct NewToken {
    pub token: String,
    pub hash: String,
    pub hint: String,
}

pub(super) fn generate() -> Result<NewToken, CoreError> {
    let mut bytes = [0u8; RANDOM_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        CoreError::StorageIo(std::io::Error::other(format!(
            "sin aleatoriedad del sistema: {error}"
        )))
    })?;
    let mut token = String::with_capacity(TOKEN_LEN);
    token.push_str(PREFIX);
    token.push_str(&hex(&bytes));
    let hint = token[token.len() - HINT_LEN..].to_owned();
    Ok(NewToken {
        hash: hash(&token),
        hint,
        token,
    })
}

/// `true` si tiene la forma de un token (sin tocar la base). Rechaza pronto entradas absurdas.
pub(super) fn well_formed(token: &str) -> bool {
    token.len() == TOKEN_LEN
        && token.starts_with(PREFIX)
        && token[PREFIX.len()..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_well_formed_unique_and_hashed() {
        let a = generate().unwrap();
        let b = generate().unwrap();
        assert!(well_formed(&a.token));
        assert_ne!(a.token, b.token);
        assert_eq!(a.hash, hash(&a.token));
        assert_eq!(a.hash.len(), 64);
        assert!(!a.hash.contains(&a.token[PREFIX.len()..]));
        assert!(a.token.ends_with(&a.hint));
    }

    #[test]
    fn malformed_tokens_are_rejected() {
        let good = generate().unwrap().token;
        assert!(!well_formed(""));
        assert!(!well_formed(&good[1..]));
        assert!(!well_formed(&good.to_uppercase()));
        assert!(!well_formed(&format!("{good}0")));
        assert!(!well_formed(&good.replace(PREFIX, "kbix_")));
    }
}
