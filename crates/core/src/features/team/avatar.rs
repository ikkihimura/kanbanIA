//! Avatar por piezas: cada campo es el índice de una pieza o color del catálogo de la UI
//! (sprites originales de 16×16 y paleta limitada). El núcleo solo valida los rangos; los
//! límites se exportan a `contracts/generated/avatarLimits.json` para que la UI no diverja.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::CoreError;

/// Número de opciones de cada pieza. La UI debe tener exactamente estas (prueba en ambos lados).
pub const AVATAR_LIMITS: AvatarLimits = AvatarLimits {
    head: 4,
    skin: 8,
    eyes: 6,
    mouth: 5,
    hair: 8,
    hair_color: 10,
    accessory: 6,
    background: 10,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarLimits {
    pub head: u8,
    pub skin: u8,
    pub eyes: u8,
    pub mouth: u8,
    pub hair: u8,
    pub hair_color: u8,
    pub accessory: u8,
    pub background: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct AvatarSpec {
    pub head: u8,
    pub skin: u8,
    pub eyes: u8,
    pub mouth: u8,
    pub hair: u8,
    pub hair_color: u8,
    pub accessory: u8,
    pub background: u8,
}

impl AvatarSpec {
    /// Avatar inicial de un agente: cabeza de robot, visor y fondo propio.
    pub const DEFAULT_AGENT: Self = Self {
        head: 2,
        skin: 6,
        eyes: 4,
        mouth: 3,
        hair: 6,
        hair_color: 7,
        accessory: 5,
        background: 3,
    };

    pub(crate) fn validate(self) -> Result<Self, CoreError> {
        let limits = AVATAR_LIMITS;
        let checks = [
            ("head", self.head, limits.head),
            ("skin", self.skin, limits.skin),
            ("eyes", self.eyes, limits.eyes),
            ("mouth", self.mouth, limits.mouth),
            ("hair", self.hair, limits.hair),
            ("hairColor", self.hair_color, limits.hair_color),
            ("accessory", self.accessory, limits.accessory),
            ("background", self.background, limits.background),
        ];
        if let Some((part, _, max)) = checks.iter().find(|(_, value, max)| value >= max) {
            return Err(CoreError::Validation {
                field: "avatar",
                reason: format!(
                    "Pieza de avatar no válida ({part}): el máximo es {}.",
                    max - 1
                ),
            });
        }
        Ok(self)
    }

    pub(crate) fn to_json(self) -> Result<String, CoreError> {
        Ok(serde_json::to_string(&self)?)
    }

    pub(crate) fn from_json(raw: &str) -> Result<Self, CoreError> {
        let spec: Self = serde_json::from_str(raw)?;
        spec.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_out_of_range_parts_and_unknown_fields() {
        assert!(AvatarSpec::DEFAULT_AGENT.validate().is_ok());
        let bad = AvatarSpec {
            eyes: AVATAR_LIMITS.eyes,
            ..AvatarSpec::DEFAULT_AGENT
        };
        assert!(matches!(
            bad.validate(),
            Err(CoreError::Validation {
                field: "avatar",
                ..
            })
        ));
        assert!(AvatarSpec::from_json(r#"{"head":0,"extra":1}"#).is_err());
    }

    #[test]
    fn default_user_avatar_in_migration_is_valid() {
        let migration = include_str!("../../../migrations/0006_team.sql");
        let start = migration.find("'{").unwrap() + 1;
        let end = migration[start..].find("}'").unwrap() + start + 1;
        AvatarSpec::from_json(&migration[start..end]).unwrap();
    }

    /// Exporta los límites junto a los tipos generados (como ts-rs) para la prueba de la UI.
    #[test]
    fn export_avatar_limits() {
        let Ok(dir) = std::env::var("TS_RS_EXPORT_DIR") else {
            return;
        };
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = manifest.join("../..").join(dir).join("avatarLimits.json");
        let json = serde_json::to_string_pretty(&AVATAR_LIMITS).unwrap() + "\n";
        if std::fs::read_to_string(&path).ok().as_deref() != Some(json.as_str()) {
            std::fs::write(&path, json).unwrap();
        }
    }
}
