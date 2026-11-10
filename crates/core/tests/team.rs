//! Pruebas de HU-008 a través de la API pública del núcleo, con SQLite temporal real.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use kanbania_core::error::ErrorCode;
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::relations::{
    SetStoryAgentInput, get_story_relations, set_story_agent,
};
use kanbania_core::features::stories::{
    ArchiveStoryInput, CreateStoryInput, StorySummary, archive_story, create_story, list_stories,
};
use kanbania_core::features::team::{
    AVATAR_LIMITS, Agent, AgentFields, AvatarSpec, Capability, CreateAgentInput, UpdateAgentInput,
    UpdateUserProfileInput, create_agent, get_user_profile, list_agents, update_agent,
    update_user_profile,
};
use kanbania_core::platform::storage::Store;

const ACTOR: &str = "ui";

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn fields(name: &str) -> AgentFields {
    AgentFields {
        name: name.into(),
        role: "Implementación backend".into(),
        provider: "Anthropic".into(),
        model: "claude-opus-5-5".into(),
        capabilities: vec![Capability::Implement, Capability::Test],
        avatar: AvatarSpec::DEFAULT_AGENT,
    }
}

async fn agent(store: &Store, name: &str) -> Agent {
    create_agent(
        store,
        ACTOR,
        &CreateAgentInput {
            request_id: rid(),
            fields: fields(name),
        },
    )
    .await
    .unwrap()
}

async fn project_with_story(store: &Store) -> (Project, StorySummary) {
    let p = create_project(
        store,
        ACTOR,
        &CreateProjectInput {
            request_id: rid(),
            name: "Equipo".into(),
        },
    )
    .await
    .unwrap();
    let s = create_story(
        store,
        ACTOR,
        &CreateStoryInput {
            request_id: rid(),
            project_id: p.id.clone(),
            title: "Tarea".into(),
            column_id: None,
            priority: None,
        },
    )
    .await
    .unwrap();
    (p, s)
}

fn assign(p: &Project, s: &StorySummary, agent: Option<&Agent>) -> SetStoryAgentInput {
    SetStoryAgentInput {
        project_id: p.id.clone(),
        story_id: s.id.clone(),
        agent_id: agent.map(|a| a.id.clone()),
    }
}

// ---------- CA-1: avatar por piezas, nombre y persistencia; usuario ≠ agente ----------

#[tokio::test]
async fn user_profile_and_avatar_persist_after_restart() {
    let data = tempfile::tempdir().unwrap();
    let custom = AvatarSpec {
        head: 1,
        skin: 3,
        eyes: 2,
        mouth: 4,
        hair: 5,
        hair_color: 9,
        accessory: 1,
        background: 7,
    };
    {
        let store = Store::open(data.path()).await.unwrap();
        let initial = get_user_profile(&store).await.unwrap();
        assert_eq!(initial.name, "Yo");
        let saved = update_user_profile(
            &store,
            &UpdateUserProfileInput {
                expected_revision: initial.revision,
                name: "  Ikki  ".into(),
                avatar: custom,
            },
        )
        .await
        .unwrap();
        assert_eq!((saved.name.as_str(), saved.revision), ("Ikki", 2));
        store.close().await;
    }

    let store = Store::open(data.path()).await.unwrap();
    let profile = get_user_profile(&store).await.unwrap();
    assert_eq!((profile.name.as_str(), profile.avatar), ("Ikki", custom));
    // El usuario no aparece entre los agentes.
    assert!(list_agents(&store).await.unwrap().is_empty());
}

#[tokio::test]
async fn invalid_avatar_and_stale_profile_are_rejected() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let profile = get_user_profile(&store).await.unwrap();

    let bad_avatar = UpdateUserProfileInput {
        expected_revision: profile.revision,
        name: "Ikki".into(),
        avatar: AvatarSpec {
            background: AVATAR_LIMITS.background,
            ..profile.avatar
        },
    };
    let error = update_user_profile(&store, &bad_avatar)
        .await
        .unwrap_err()
        .to_payload(None);
    assert_eq!(
        (error.code, error.field.as_deref()),
        (ErrorCode::Validation, Some("avatar"))
    );

    let stale = UpdateUserProfileInput {
        expected_revision: 99,
        name: "Ikki".into(),
        avatar: profile.avatar,
    };
    assert_eq!(
        update_user_profile(&store, &stale)
            .await
            .unwrap_err()
            .code(),
        ErrorCode::Conflict
    );
}

// ---------- CA-2: nombre, rol, proveedor, modelo y capacidad; cambiar modelo conserva identidad ----------

#[tokio::test]
async fn agent_fields_are_separate_and_model_change_keeps_identity() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let original = agent(&store, "Robota").await;
    assert_eq!(original.number, 1);
    assert_eq!(
        original.capabilities,
        [Capability::Implement, Capability::Test]
    );

    let mut changed = fields("Robota");
    changed.provider = "Otro proveedor".into();
    changed.model = "modelo-nuevo-2".into();
    let updated = update_agent(
        &store,
        &UpdateAgentInput {
            id: original.id.clone(),
            expected_revision: original.revision,
            fields: changed,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.id, original.id);
    assert_eq!(updated.number, original.number);
    assert_eq!(updated.name, original.name);
    assert_eq!(
        updated.avatar, original.avatar,
        "el avatar no cambia con el modelo"
    );
    assert_eq!(
        (updated.provider.as_str(), updated.model.as_str()),
        ("Otro proveedor", "modelo-nuevo-2")
    );
    assert_eq!(updated.role, original.role);
    assert_eq!(
        list_agents(&store).await.unwrap().len(),
        1,
        "no se creó otro agente"
    );
}

#[tokio::test]
async fn agent_names_are_unique_case_insensitively_and_fields_validated() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    agent(&store, "Robota").await;
    let other = agent(&store, "Pixel").await;

    let duplicate = create_agent(
        &store,
        ACTOR,
        &CreateAgentInput {
            request_id: rid(),
            fields: fields("ROBOTA"),
        },
    )
    .await
    .unwrap_err()
    .to_payload(None);
    assert_eq!(
        (duplicate.code, duplicate.field.as_deref()),
        (ErrorCode::AlreadyExists, Some("name"))
    );

    let rename_clash = update_agent(
        &store,
        &UpdateAgentInput {
            id: other.id.clone(),
            expected_revision: other.revision,
            fields: fields("robota"),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(rename_clash.code(), ErrorCode::AlreadyExists);

    for (bad, field) in [
        (
            AgentFields {
                name: "  ".into(),
                ..fields("x")
            },
            "name",
        ),
        (
            AgentFields {
                model: "m".repeat(81),
                ..fields("x")
            },
            "model",
        ),
        (
            AgentFields {
                role: "a\nb".into(),
                ..fields("x")
            },
            "role",
        ),
    ] {
        let error = create_agent(
            &store,
            ACTOR,
            &CreateAgentInput {
                request_id: rid(),
                fields: bad,
            },
        )
        .await
        .unwrap_err()
        .to_payload(None);
        assert_eq!(error.field.as_deref(), Some(field));
    }

    // Capacidades duplicadas se normalizan.
    let mut repeated = fields("Dup");
    repeated.capabilities = vec![Capability::Review, Capability::Plan, Capability::Review];
    let created = create_agent(
        &store,
        ACTOR,
        &CreateAgentInput {
            request_id: rid(),
            fields: repeated,
        },
    )
    .await
    .unwrap();
    assert_eq!(created.capabilities, [Capability::Plan, Capability::Review]);
}

// ---------- CA-3: asignar sin simular actividad ----------

#[tokio::test]
async fn assigning_an_agent_records_only_the_assignment() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let (p, s) = project_with_story(&store).await;
    let robota = agent(&store, "Robota").await;
    let before = list_stories(&store, &p.id, false).await.unwrap().remove(0);

    let relations = set_story_agent(&store, &assign(&p, &s, Some(&robota)))
        .await
        .unwrap();
    assert_eq!(relations.agent.as_ref().unwrap().name, "Robota");
    // Idempotente.
    set_story_agent(&store, &assign(&p, &s, Some(&robota)))
        .await
        .unwrap();

    let after = list_stories(&store, &p.id, false).await.unwrap().remove(0);
    assert_eq!(after.agent_id.as_deref(), Some(robota.id.as_str()));
    // Sin efectos secundarios: misma columna, posición y revisión.
    assert_eq!(
        (&after.column_id, after.position, after.revision),
        (&before.column_id, before.position, before.revision)
    );
    assert_eq!(list_agents(&store).await.unwrap()[0].assigned_stories, 1);

    let cleared = set_story_agent(&store, &assign(&p, &s, None))
        .await
        .unwrap();
    assert!(cleared.agent.is_none());
    assert_eq!(list_agents(&store).await.unwrap()[0].assigned_stories, 0);
}

#[tokio::test]
async fn reassignment_survives_model_change_and_restart() {
    let data = tempfile::tempdir().unwrap();
    let (project_id, story_id, agent_id) = {
        let store = Store::open(data.path()).await.unwrap();
        let (p, s) = project_with_story(&store).await;
        let robota = agent(&store, "Robota").await;
        set_story_agent(&store, &assign(&p, &s, Some(&robota)))
            .await
            .unwrap();
        let mut new_model = fields("Robota");
        new_model.model = "otro-modelo".into();
        update_agent(
            &store,
            &UpdateAgentInput {
                id: robota.id.clone(),
                expected_revision: robota.revision,
                fields: new_model,
            },
        )
        .await
        .unwrap();
        store.close().await;
        (p.id, s.id, robota.id)
    };

    let store = Store::open(data.path()).await.unwrap();
    let relations = get_story_relations(&store, &project_id, &story_id)
        .await
        .unwrap();
    assert_eq!(
        relations.agent.unwrap().id,
        agent_id,
        "la asignación sigue al mismo agente"
    );
}

#[tokio::test]
async fn assignment_rejects_unknown_agent_and_archived_story() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let (p, s) = project_with_story(&store).await;
    let robota = agent(&store, "Robota").await;

    let unknown = SetStoryAgentInput {
        agent_id: Some("no-existe".into()),
        ..assign(&p, &s, None)
    };
    assert_eq!(
        set_story_agent(&store, &unknown).await.unwrap_err().code(),
        ErrorCode::NotFound
    );

    archive_story(
        &store,
        &ArchiveStoryInput {
            project_id: p.id.clone(),
            id: s.id.clone(),
            expected_revision: s.revision,
        },
    )
    .await
    .unwrap();
    let archived = set_story_agent(&store, &assign(&p, &s, Some(&robota)))
        .await
        .unwrap_err();
    assert_eq!(archived.code(), ErrorCode::Validation);
}

#[tokio::test]
async fn failed_reassignment_keeps_previous_agent_and_same_agent_keeps_timestamp() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let (p, s) = project_with_story(&store).await;
    let robota = agent(&store, "Robota").await;
    set_story_agent(&store, &assign(&p, &s, Some(&robota)))
        .await
        .unwrap();
    let assigned_at = || async {
        let id = s.id.clone();
        store
            .write(move |conn| {
                Box::pin(async move {
                    Ok(sqlx::query_scalar::<_, String>(
                        "SELECT assigned_at FROM story_agents WHERE story_id = ?",
                    )
                    .bind(id)
                    .fetch_one(&mut *conn)
                    .await?)
                })
            })
            .await
            .unwrap()
    };
    let first = assigned_at().await;

    let unknown = SetStoryAgentInput {
        agent_id: Some("no-existe".into()),
        ..assign(&p, &s, None)
    };
    assert_eq!(
        set_story_agent(&store, &unknown).await.unwrap_err().code(),
        ErrorCode::NotFound
    );
    let rel = get_story_relations(&store, &p.id, &s.id).await.unwrap();
    assert_eq!(
        rel.agent.unwrap().id,
        robota.id,
        "la asignación previa se conserva"
    );

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    set_story_agent(&store, &assign(&p, &s, Some(&robota)))
        .await
        .unwrap();
    assert_eq!(
        assigned_at().await,
        first,
        "reasignar el mismo agente no cambia nada"
    );

    let pixel = agent(&store, "Pixel").await;
    set_story_agent(&store, &assign(&p, &s, Some(&pixel)))
        .await
        .unwrap();
    assert_ne!(
        assigned_at().await,
        first,
        "otro agente es una asignación nueva"
    );
}
