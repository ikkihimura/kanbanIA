//! Credenciales de agentes (HU-012) a través de la API pública del núcleo, con SQLite real.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use kanbania_core::error::ErrorCode;
use kanbania_core::features::access::{
    CreateAgentCredentialInput, CreatedAgentCredential, MAX_ACTIVE_PER_PROJECT,
    RevokeAgentCredentialInput, authenticate_agent, create_agent_credential,
    list_agent_credentials, revoke_agent_credential,
};
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::team::{
    Agent, AgentFields, AvatarSpec, CreateAgentInput, create_agent,
};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::storage::Store;

const ACTOR: &str = "ui";

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

async fn project(store: &Store, name: &str) -> Project {
    create_project(
        store,
        ACTOR,
        &CreateProjectInput {
            request_id: rid(),
            name: name.into(),
        },
    )
    .await
    .unwrap()
}

async fn agent(store: &Store, name: &str) -> Agent {
    create_agent(
        store,
        ACTOR,
        &CreateAgentInput {
            request_id: rid(),
            fields: AgentFields {
                name: name.into(),
                role: String::new(),
                provider: String::new(),
                model: String::new(),
                capabilities: vec![],
                avatar: AvatarSpec::DEFAULT_AGENT,
            },
        },
    )
    .await
    .unwrap()
}

async fn credential(store: &Store, p: &Project, a: &Agent) -> CreatedAgentCredential {
    create_agent_credential(
        store,
        &Actor::Human,
        &CreateAgentCredentialInput {
            project_id: p.id.clone(),
            agent_id: a.id.clone(),
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_credential_authenticates_its_agent_in_its_project_only() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let (p, other) = (project(&store, "Uno").await, project(&store, "Dos").await);
    let a = agent(&store, "Robo").await;

    let created = credential(&store, &p, &a).await;
    assert_eq!(created.credential.agent_name, "Robo");
    assert!(created.credential.last_used_at.is_none());
    assert!(created.token.ends_with(&created.credential.token_hint));
    // El Debug no filtra el token (logs).
    assert!(!format!("{created:?}").contains(&created.token));

    let session = authenticate_agent(&store, &created.token).await.unwrap();
    assert_eq!(session.project_id(), p.id);
    assert_eq!(session.agent_id(), a.id);
    assert_eq!(
        session.actor(),
        Actor::Agent {
            agent_id: a.id.clone()
        }
    );

    let listed = list_agent_credentials(&store, &p.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].last_used_at.is_some(), "registra el último uso");
    // No escribe en cada petición: dentro del minuto, el último uso no cambia.
    authenticate_agent(&store, &created.token).await.unwrap();
    let again = list_agent_credentials(&store, &p.id).await.unwrap();
    assert_eq!(again[0].last_used_at, listed[0].last_used_at);
    assert!(
        list_agent_credentials(&store, &other.id)
            .await
            .unwrap()
            .is_empty()
    );

    // La lista nunca lleva el token ni su hash.
    let json = serde_json::to_string(&listed).unwrap();
    assert!(!json.contains(&created.token));
    assert!(!json.to_lowercase().contains("hash"));
}

#[tokio::test]
async fn the_token_is_never_stored_in_clear() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let (p, a) = (project(&store, "Uno").await, agent(&store, "Robo").await);
    let created = credential(&store, &p, &a).await;
    store.close().await;
    // Suelta el bloqueo del directorio de datos: en Windows impide leer su archivo.
    drop(store);

    // Ni en la base ni en el WAL ni en ningún otro archivo del directorio de datos.
    let secret = created.token.trim_start_matches("kbia_");
    for entry in walk(data.path()) {
        let bytes = std::fs::read(&entry).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains(secret),
            "token en claro en {}",
            entry.display()
        );
    }
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[tokio::test]
async fn invalid_unknown_and_revoked_tokens_are_unauthorized() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let (p, a) = (project(&store, "Uno").await, agent(&store, "Robo").await);
    let created = credential(&store, &p, &a).await;

    let unknown = format!("kbia_{}", "0".repeat(64));
    let mut tampered = created.token.clone();
    tampered.pop();
    tampered.push(if created.token.ends_with('0') {
        '1'
    } else {
        '0'
    });
    for bad in [
        "",
        "Bearer x",
        unknown.as_str(),
        tampered.as_str(),
        &created.token.to_uppercase(),
        &format!(" {}", created.token),
    ] {
        let error = authenticate_agent(&store, bad).await.unwrap_err();
        assert_eq!(
            error.to_payload(None).code,
            ErrorCode::Unauthorized,
            "{bad:?}"
        );
    }

    let revoked = revoke_agent_credential(
        &store,
        &Actor::Human,
        &RevokeAgentCredentialInput {
            project_id: p.id.clone(),
            id: created.credential.id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(revoked.revoked_at.is_some());
    let error = authenticate_agent(&store, &created.token)
        .await
        .unwrap_err();
    assert_eq!(error.to_payload(None).code, ErrorCode::Unauthorized);

    // Revocar otra vez no cambia la fecha.
    let again = revoke_agent_credential(
        &store,
        &Actor::Human,
        &RevokeAgentCredentialInput {
            project_id: p.id.clone(),
            id: created.credential.id.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(again.revoked_at, revoked.revoked_at);
}

#[tokio::test]
async fn only_people_manage_credentials_and_only_within_the_project() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let (p, other) = (project(&store, "Uno").await, project(&store, "Dos").await);
    let a = agent(&store, "Robo").await;
    let created = credential(&store, &p, &a).await;
    let as_agent = Actor::Agent {
        agent_id: a.id.clone(),
    };

    let error = create_agent_credential(
        &store,
        &as_agent,
        &CreateAgentCredentialInput {
            project_id: p.id.clone(),
            agent_id: a.id.clone(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_payload(None).code, ErrorCode::Forbidden);
    let error = revoke_agent_credential(
        &store,
        &as_agent,
        &RevokeAgentCredentialInput {
            project_id: p.id.clone(),
            id: created.credential.id.clone(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_payload(None).code, ErrorCode::Forbidden);

    // Revocar desde otro proyecto: no encontrada y sigue activa.
    let error = revoke_agent_credential(
        &store,
        &Actor::Human,
        &RevokeAgentCredentialInput {
            project_id: other.id.clone(),
            id: created.credential.id.clone(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_payload(None).code, ErrorCode::NotFound);
    authenticate_agent(&store, &created.token).await.unwrap();

    // Agente o proyecto inexistentes.
    for (project_id, agent_id) in [(p.id.clone(), rid()), (rid(), a.id.clone())] {
        let error = create_agent_credential(
            &store,
            &Actor::Human,
            &CreateAgentCredentialInput {
                project_id,
                agent_id,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_payload(None).code, ErrorCode::NotFound);
    }
}

#[tokio::test]
async fn active_credentials_per_project_are_limited() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let (p, a) = (project(&store, "Uno").await, agent(&store, "Robo").await);
    let mut first = None;
    for _ in 0..MAX_ACTIVE_PER_PROJECT {
        let created = credential(&store, &p, &a).await;
        first.get_or_insert(created.credential.id);
    }
    let input = CreateAgentCredentialInput {
        project_id: p.id.clone(),
        agent_id: a.id.clone(),
    };
    let error = create_agent_credential(&store, &Actor::Human, &input)
        .await
        .unwrap_err();
    assert_eq!(error.to_payload(None).code, ErrorCode::Validation);

    // Revocar libera un hueco; las revocadas siguen en la lista, al final.
    revoke_agent_credential(
        &store,
        &Actor::Human,
        &RevokeAgentCredentialInput {
            project_id: p.id.clone(),
            id: first.clone().unwrap(),
        },
    )
    .await
    .unwrap();
    create_agent_credential(&store, &Actor::Human, &input)
        .await
        .unwrap();
    let listed = list_agent_credentials(&store, &p.id).await.unwrap();
    assert_eq!(listed.len() as i64, MAX_ACTIVE_PER_PROJECT + 1);
    assert_eq!(listed.last().unwrap().id, first.unwrap());
}

// ---------- Copias: restaurar no reactiva credenciales revocadas ----------

use kanbania_core::platform::storage::{BackupKind, backup_path, restore_backup, save_backup_copy};

#[tokio::test]
async fn restoring_an_old_backup_keeps_later_revocations() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let (p, a) = (project(&store, "Uno").await, agent(&store, "Robo").await);
    let leaked = credential(&store, &p, &a).await;
    let kept = credential(&store, &p, &a).await;
    let backup = store.create_backup(BackupKind::Manual).await.unwrap();
    revoke_agent_credential(
        &store,
        &Actor::Human,
        &RevokeAgentCredentialInput {
            project_id: p.id.clone(),
            id: leaked.credential.id.clone(),
        },
    )
    .await
    .unwrap();
    let after_backup = credential(&store, &p, &a).await;
    store.close().await;
    drop(store);

    restore_backup(data.path(), &backup_path(data.path(), &backup.id).unwrap())
        .await
        .unwrap();
    let store = Store::open(data.path()).await.unwrap();
    // La revocada después de la copia sigue revocada; la que no se tocó sigue valiendo; la
    // creada después de la copia no existe en ella.
    let error = authenticate_agent(&store, &leaked.token).await.unwrap_err();
    assert_eq!(error.to_payload(None).code, ErrorCode::Unauthorized);
    authenticate_agent(&store, &kept.token).await.unwrap();
    let error = authenticate_agent(&store, &after_backup.token)
        .await
        .unwrap_err();
    assert_eq!(error.to_payload(None).code, ErrorCode::Unauthorized);
}

#[tokio::test]
async fn restoring_into_another_profile_revokes_every_credential_of_the_copy() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let (p, a) = (project(&store, "Uno").await, agent(&store, "Robo").await);
    let created = credential(&store, &p, &a).await;
    let backup = store.create_backup(BackupKind::Manual).await.unwrap();
    let outside = tempfile::tempdir().unwrap();
    let folder = save_backup_copy(data.path(), &backup.id, outside.path()).unwrap();
    store.close().await;

    // Perfil nuevo (otro equipo): no se sabe qué se revocó, así que nada de la copia autentica.
    let fresh = tempfile::tempdir().unwrap();
    restore_backup(fresh.path(), &outside.path().join(folder))
        .await
        .unwrap();
    let store = Store::open(fresh.path()).await.unwrap();
    let error = authenticate_agent(&store, &created.token)
        .await
        .unwrap_err();
    assert_eq!(error.to_payload(None).code, ErrorCode::Unauthorized);
    let listed = list_agent_credentials(&store, &p.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].revoked_at.is_some(), "aparece como revocada");
}
