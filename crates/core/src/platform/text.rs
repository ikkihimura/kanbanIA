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

#[cfg(test)]
mod tests {
    use super::has_invisible_formatting;

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
