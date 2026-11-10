use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::avatar::AvatarSpec;
use super::validate_text;
use crate::error::CoreError;
use crate::platform::storage::Store;

const ENTITY: &str = "Perfil";

/// Usuario local (persona). Es una entidad distinta de los agentes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UserProfile {
    pub name: String,
    pub avatar: AvatarSpec,
    #[ts(type = "number")]
    pub revision: i64,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateUserProfileInput {
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub name: String,
    pub avatar: AvatarSpec,
}

async fn load(conn: &mut sqlx::SqliteConnection) -> Result<UserProfile, CoreError> {
    let (name, avatar, revision): (String, String, i64) =
        sqlx::query_as("SELECT name, avatar, revision FROM user_profile WHERE id = 1")
            .fetch_optional(&mut *conn)
            .await?
            .ok_or(CoreError::NotFound { entity: ENTITY })?;
    Ok(UserProfile {
        name,
        avatar: AvatarSpec::from_json(&avatar)?,
        revision,
    })
}

pub async fn get_user_profile(store: &Store) -> Result<UserProfile, CoreError> {
    let mut conn = store.reader().acquire().await?;
    load(&mut conn).await
}

pub async fn update_user_profile(
    store: &Store,
    input: &UpdateUserProfileInput,
) -> Result<UserProfile, CoreError> {
    let name = validate_text("name", &input.name, 1, 60, "El nombre")?;
    let avatar = input.avatar.validate()?.to_json()?;
    let expected = input.expected_revision;

    store
        .write(move |conn| {
            Box::pin(async move {
                let current = load(conn).await?;
                if current.revision != expected {
                    return Err(CoreError::Conflict {
                        current: serde_json::to_value(&current)?,
                    });
                }
                sqlx::query(
                    "UPDATE user_profile SET name = ?, avatar = ?, revision = revision + 1, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE id = 1 AND revision = ?",
                )
                .bind(&name)
                .bind(&avatar)
                .bind(expected)
                .execute(&mut *conn)
                .await?;
                load(conn).await
            })
        })
        .await
}
