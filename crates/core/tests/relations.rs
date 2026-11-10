//! Pruebas de HU-006 a través de la API pública del núcleo, con SQLite temporal real.
//!
//! La aceptación (HU-009) aún no tiene operación: estas pruebas fijan `accepted_at` con SQL
//! para verificar avance de épicas, prerequisitos y reapertura.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::path::Path;

use kanbania_core::error::{CoreError, ErrorCode};
use kanbania_core::features::epics::{
    CreateEpicInput, Epic, UpdateEpicInput, create_epic, list_epics, update_epic,
};
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::relations::{
    DependencyInput, RelationState, SetStoryEpicInput, SetStoryParentInput, StoryRelations,
    add_story_dependency, get_story_relations, remove_story_dependency, set_story_epic,
    set_story_parent,
};
use kanbania_core::features::stories::{
    ArchiveStoryInput, CreateStoryInput, StorySummary, archive_story, create_story, list_stories,
};
use kanbania_core::platform::storage::Store;

const ACTOR: &str = "ui";

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

struct Env {
    _data: tempfile::TempDir,
    store: Store,
}

impl Env {
    async fn new() -> Self {
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).await.unwrap();
        Self { _data: data, store }
    }

    async fn project(&self, name: &str) -> Project {
        create_project(
            &self.store,
            ACTOR,
            &CreateProjectInput {
                request_id: rid(),
                name: name.into(),
            },
        )
        .await
        .unwrap()
    }

    async fn story(&self, p: &Project, title: &str) -> StorySummary {
        create_story(
            &self.store,
            ACTOR,
            &CreateStoryInput {
                request_id: rid(),
                project_id: p.id.clone(),
                title: title.into(),
                column_id: None,
                priority: None,
            },
        )
        .await
        .unwrap()
    }

    async fn epic(&self, p: &Project, title: &str) -> Epic {
        create_epic(
            &self.store,
            ACTOR,
            &CreateEpicInput {
                request_id: rid(),
                project_id: p.id.clone(),
                title: title.into(),
                description: None,
            },
        )
        .await
        .unwrap()
    }

    async fn epic_by_id(&self, p: &Project, id: &str) -> Epic {
        list_epics(&self.store, &p.id)
            .await
            .unwrap()
            .into_iter()
            .find(|e| e.id == id)
            .unwrap()
    }

    /// Simula la aceptación (HU-009) o su reapertura.
    async fn set_accepted(&self, story: &StorySummary, accepted: bool) {
        let id = story.id.clone();
        self.store
            .write(move |conn| {
                Box::pin(async move {
                    sqlx::query(
                        "UPDATE stories SET accepted_at = \
                         CASE WHEN ? THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END WHERE id = ?",
                    )
                    .bind(accepted)
                    .bind(id)
                    .execute(&mut *conn)
                    .await?;
                    Ok(())
                })
            })
            .await
            .unwrap();
    }

    async fn join_epic(
        &self,
        s: &StorySummary,
        epic: Option<&Epic>,
    ) -> Result<StoryRelations, CoreError> {
        set_story_epic(
            &self.store,
            &SetStoryEpicInput {
                project_id: s.project_id.clone(),
                story_id: s.id.clone(),
                epic_id: epic.map(|e| e.id.clone()),
            },
        )
        .await
    }

    async fn parent(
        &self,
        child: &StorySummary,
        parent: Option<&StorySummary>,
    ) -> Result<StoryRelations, CoreError> {
        set_story_parent(
            &self.store,
            &SetStoryParentInput {
                project_id: child.project_id.clone(),
                story_id: child.id.clone(),
                parent_id: parent.map(|p| p.id.clone()),
            },
        )
        .await
    }

    async fn depend(
        &self,
        s: &StorySummary,
        on: &StorySummary,
    ) -> Result<StoryRelations, CoreError> {
        add_story_dependency(&self.store, &dep(s, &on.id)).await
    }

    async fn summary(&self, s: &StorySummary) -> StorySummary {
        list_stories(&self.store, &s.project_id, true)
            .await
            .unwrap()
            .into_iter()
            .find(|x| x.id == s.id)
            .unwrap()
    }
}

fn dep(s: &StorySummary, on_id: &str) -> DependencyInput {
    DependencyInput {
        project_id: s.project_id.clone(),
        story_id: s.id.clone(),
        depends_on_id: on_id.to_owned(),
    }
}

fn numbers(refs: &[kanbania_core::features::relations::StoryRef]) -> Vec<i64> {
    refs.iter().map(|r| r.number).collect()
}

// ---------- CA-1: épicas, asociación y subtareas con IDs estables ----------

#[tokio::test]
async fn epics_have_stable_numbers_and_revision() {
    let env = Env::new().await;
    let p = env.project("Épicas").await;
    let e1 = env.epic(&p, "Pagos").await;
    let e2 = env.epic(&p, "Informes").await;
    assert_eq!((e1.number, e2.number), (1, 2));

    let input = UpdateEpicInput {
        project_id: p.id.clone(),
        id: e1.id.clone(),
        expected_revision: 1,
        title: "Pagos y facturas".into(),
        description: "Objetivo **trimestral**".into(),
    };
    let updated = update_epic(&env.store, &input).await.unwrap();
    assert_eq!(updated.revision, 2);
    let stale = update_epic(&env.store, &input).await.unwrap_err();
    assert_eq!(stale.code(), ErrorCode::Conflict);
}

#[tokio::test]
async fn stories_join_epics_and_subtasks_persist_after_restart() {
    let data = tempfile::tempdir().unwrap();
    let (project_id, parent_id, child_id, epic_id) = {
        let store = Store::open(data.path()).await.unwrap();
        let env = Env {
            _data: tempfile::tempdir().unwrap(),
            store: store.clone(),
        };
        let p = env.project("Persistencia").await;
        let epic = env.epic(&p, "Onboarding").await;
        let parent = env.story(&p, "Registro").await;
        let child = env.story(&p, "Validar correo").await;
        env.join_epic(&parent, Some(&epic)).await.unwrap();
        // Idempotente: repetir no cambia nada ni falla.
        env.join_epic(&parent, Some(&epic)).await.unwrap();
        env.parent(&child, Some(&parent)).await.unwrap();
        env.parent(&child, Some(&parent)).await.unwrap();
        store.close().await;
        (p.id, parent.id, child.id, epic.id)
    };

    let store = Store::open(data.path()).await.unwrap();
    let relations = get_story_relations(&store, &project_id, &parent_id)
        .await
        .unwrap();
    assert_eq!(relations.epic.unwrap().id, epic_id);
    assert_eq!(relations.children.len(), 1);
    assert_eq!(relations.children[0].id, child_id);
    let child = get_story_relations(&store, &project_id, &child_id)
        .await
        .unwrap();
    assert_eq!(child.parent.unwrap().id, parent_id);

    let listed = list_stories(&store, &project_id, false).await.unwrap();
    let child_summary = listed.iter().find(|s| s.id == child_id).unwrap();
    assert_eq!(child_summary.parent_id.as_deref(), Some(parent_id.as_str()));
    let parent_summary = listed.iter().find(|s| s.id == parent_id).unwrap();
    assert_eq!(parent_summary.epic_id.as_deref(), Some(epic_id.as_str()));
}

#[tokio::test]
async fn relations_can_be_cleared_and_changed() {
    let env = Env::new().await;
    let p = env.project("Cambios").await;
    let (a, b) = (env.epic(&p, "A").await, env.epic(&p, "B").await);
    let s = env.story(&p, "Historia").await;
    env.join_epic(&s, Some(&a)).await.unwrap();
    let moved = env.join_epic(&s, Some(&b)).await.unwrap();
    assert_eq!(moved.epic.unwrap().id, b.id);
    let cleared = env.join_epic(&s, None).await.unwrap();
    assert!(cleared.epic.is_none());

    let parent = env.story(&p, "Padre").await;
    env.parent(&s, Some(&parent)).await.unwrap();
    assert!(env.parent(&s, None).await.unwrap().parent.is_none());
}

// ---------- CA-2: ciclos y relaciones entre proyectos ----------

#[tokio::test]
async fn parent_cycles_are_rejected_direct_and_indirect() {
    let env = Env::new().await;
    let p = env.project("Árbol").await;
    let a = env.story(&p, "A").await;
    let b = env.story(&p, "B").await;
    let c = env.story(&p, "C").await;

    let own = env.parent(&a, Some(&a)).await.unwrap_err().to_payload(None);
    assert_eq!(
        (own.code, own.field.as_deref()),
        (ErrorCode::Cycle, Some("parentId"))
    );

    env.parent(&b, Some(&a)).await.unwrap(); // A → B
    env.parent(&c, Some(&b)).await.unwrap(); // A → B → C

    let direct = env.parent(&a, Some(&b)).await.unwrap_err();
    assert_eq!(direct.code(), ErrorCode::Cycle);
    let indirect = env.parent(&a, Some(&c)).await.unwrap_err().to_payload(None);
    assert_eq!(indirect.code, ErrorCode::Cycle);
    assert!(
        indirect.message.contains("#3") && indirect.message.contains("#1"),
        "{}",
        indirect.message
    );

    // Reorganizar sin ciclo sí se permite: C pasa a colgar de A.
    env.parent(&c, Some(&a)).await.unwrap();
    let children = get_story_relations(&env.store, &p.id, &a.id)
        .await
        .unwrap()
        .children;
    assert_eq!(numbers(&children), [2, 3]);
}

#[tokio::test]
async fn dependency_cycles_are_rejected_direct_and_indirect() {
    let env = Env::new().await;
    let p = env.project("Grafo").await;
    let a = env.story(&p, "A").await;
    let b = env.story(&p, "B").await;
    let c = env.story(&p, "C").await;
    let d = env.story(&p, "D").await;

    let own = env.depend(&a, &a).await.unwrap_err();
    assert_eq!(own.code(), ErrorCode::Cycle);

    env.depend(&a, &b).await.unwrap(); // A depende de B
    env.depend(&b, &c).await.unwrap(); // B depende de C
    env.depend(&c, &d).await.unwrap(); // C depende de D

    let direct = env.depend(&b, &a).await.unwrap_err().to_payload(None);
    assert_eq!(
        (direct.code, direct.field.as_deref()),
        (ErrorCode::Cycle, Some("dependsOnId"))
    );
    let indirect = env.depend(&d, &a).await.unwrap_err();
    assert_eq!(indirect.code(), ErrorCode::Cycle);

    // Un rombo (A→D además de A→B→C→D) no es ciclo.
    env.depend(&a, &d).await.unwrap();
    // Idempotente.
    let again = env.depend(&a, &d).await.unwrap();
    assert_eq!(numbers(&again.prerequisites), [2, 4]);
}

#[tokio::test]
async fn relations_across_projects_are_rejected() {
    let env = Env::new().await;
    let mine = env.project("Mío").await;
    let other = env.project("Ajeno").await;
    let s = env.story(&mine, "Local").await;
    let foreign = env.story(&other, "Externa").await;
    let foreign_epic = env.epic(&other, "Épica ajena").await;

    assert_eq!(
        env.join_epic(&s, Some(&foreign_epic))
            .await
            .unwrap_err()
            .code(),
        ErrorCode::NotFound
    );
    assert_eq!(
        env.parent(&s, Some(&foreign)).await.unwrap_err().code(),
        ErrorCode::NotFound
    );
    assert_eq!(
        env.depend(&s, &foreign).await.unwrap_err().code(),
        ErrorCode::NotFound
    );

    // La base también lo impide aunque se salte el núcleo (claves foráneas compuestas).
    for sql in [
        "INSERT INTO story_dependencies (story_id, depends_on_id, project_id) VALUES (?, ?, ?)",
        "INSERT INTO story_parents (story_id, parent_id, project_id) VALUES (?, ?, ?)",
    ] {
        let (sid, fid, pid) = (s.id.clone(), foreign.id.clone(), mine.id.clone());
        let raw = env
            .store
            .write(move |conn| {
                Box::pin(async move {
                    sqlx::query(sql)
                        .bind(sid)
                        .bind(fid)
                        .bind(pid)
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
            .await;
        assert!(matches!(raw, Err(CoreError::Database(_))), "{sql}");
    }
}

#[tokio::test]
async fn archived_stories_cannot_gain_relations_but_dependencies_can_be_removed() {
    let env = Env::new().await;
    let p = env.project("Archivo").await;
    let s = env.story(&p, "Activa").await;
    let gone = env.story(&p, "Descartada").await;
    env.depend(&s, &gone).await.unwrap();
    archive_story(
        &env.store,
        &ArchiveStoryInput {
            project_id: p.id.clone(),
            id: gone.id.clone(),
            expected_revision: gone.revision,
        },
    )
    .await
    .unwrap();

    let relations = get_story_relations(&env.store, &p.id, &s.id).await.unwrap();
    assert_eq!(relations.prerequisites[0].state, RelationState::Archived);
    assert_eq!(
        relations.pending_prerequisites, 1,
        "archivada sin aceptar sigue bloqueando"
    );

    let new_dep = env.story(&p, "Otra").await;
    assert_eq!(
        env.depend(&new_dep, &gone).await.unwrap_err().code(),
        ErrorCode::Validation
    );
    assert_eq!(
        env.parent(&new_dep, Some(&gone)).await.unwrap_err().code(),
        ErrorCode::Validation
    );

    let freed = remove_story_dependency(&env.store, &dep(&s, &gone.id))
        .await
        .unwrap();
    assert_eq!(freed.pending_prerequisites, 0);
    // Quitar de nuevo es idempotente.
    remove_story_dependency(&env.store, &dep(&s, &gone.id))
        .await
        .unwrap();
}

// ---------- CA-3: avance de épica y prerequisitos no aceptados ----------

#[tokio::test]
async fn epic_progress_counts_accepted_over_non_archived_and_reopen() {
    let env = Env::new().await;
    let p = env.project("Avance").await;
    let epic = env.epic(&p, "Lanzamiento").await;

    // Cero historias: avance indefinido, no 0 % ni 100 %.
    let empty = env.epic_by_id(&p, &epic.id).await.progress;
    assert_eq!((empty.accepted, empty.total, empty.percent), (0, 0, None));

    let mut stories = Vec::new();
    for title in ["uno", "dos", "tres", "archivada"] {
        let s = env.story(&p, title).await;
        env.join_epic(&s, Some(&epic)).await.unwrap();
        stories.push(s);
    }
    let archived = stories.pop().unwrap();
    archive_story(
        &env.store,
        &ArchiveStoryInput {
            project_id: p.id.clone(),
            id: archived.id.clone(),
            expected_revision: archived.revision,
        },
    )
    .await
    .unwrap();
    env.set_accepted(&archived, true).await; // aceptada pero archivada: no cuenta

    let progress = env.epic_by_id(&p, &epic.id).await.progress;
    assert_eq!(
        (progress.accepted, progress.total, progress.percent),
        (0, 3, Some(0))
    );

    env.set_accepted(&stories[0], true).await;
    env.set_accepted(&stories[1], true).await;
    let progress = env.epic_by_id(&p, &epic.id).await.progress;
    assert_eq!(
        (progress.accepted, progress.total, progress.percent),
        (2, 3, Some(66))
    );

    // Reapertura de una historia aceptada: el avance baja.
    env.set_accepted(&stories[1], false).await;
    let progress = env.epic_by_id(&p, &epic.id).await.progress;
    assert_eq!((progress.accepted, progress.percent), (1, Some(33)));
}

#[tokio::test]
async fn unaccepted_prerequisites_are_exposed_and_update_on_accept_and_reopen() {
    let env = Env::new().await;
    let p = env.project("Bloqueos").await;
    let feature = env.story(&p, "Funcionalidad").await;
    let api = env.story(&p, "API").await;
    let schema = env.story(&p, "Esquema").await;
    env.depend(&feature, &api).await.unwrap();
    env.depend(&feature, &schema).await.unwrap();

    assert_eq!(env.summary(&feature).await.pending_prerequisites, 2);
    let relations = get_story_relations(&env.store, &p.id, &feature.id)
        .await
        .unwrap();
    assert!(
        relations
            .prerequisites
            .iter()
            .all(|r| r.state == RelationState::Pending)
    );
    let api_side = get_story_relations(&env.store, &p.id, &api.id)
        .await
        .unwrap();
    assert_eq!(numbers(&api_side.dependents), [feature.number]);

    env.set_accepted(&api, true).await;
    assert_eq!(env.summary(&feature).await.pending_prerequisites, 1);
    env.set_accepted(&schema, true).await;
    assert_eq!(env.summary(&feature).await.pending_prerequisites, 0);

    // Reabrir un prerequisito aceptado vuelve a bloquear.
    env.set_accepted(&api, false).await;
    let relations = get_story_relations(&env.store, &p.id, &feature.id)
        .await
        .unwrap();
    assert_eq!(relations.pending_prerequisites, 1);
    let api_ref = relations
        .prerequisites
        .iter()
        .find(|r| r.id == api.id)
        .unwrap();
    assert_eq!(api_ref.state, RelationState::Pending);
}

#[tokio::test]
async fn migration_keeps_existing_stories_unrelated_and_unaccepted() {
    let data = tempfile::tempdir().unwrap();
    {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(data.path().join("kanbania.sqlite3"))
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
        let migrator =
            sqlx::migrate::Migrator::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"))
                .await
                .unwrap();
        migrator.run_to(4, &pool).await.unwrap();
        for sql in [
            "INSERT INTO projects (id, name) VALUES ('p', 'Antiguo')",
            "INSERT INTO columns (id, project_id, name, category, position) \
             VALUES ('c', 'p', 'Backlog', 'backlog', 0)",
            "INSERT INTO stories (id, project_id, number, column_id, position, title) \
             VALUES ('s', 'p', 1, 'c', 0, 'Existente')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        pool.close().await;
    }

    let store = Store::open(data.path()).await.unwrap();
    let stories = list_stories(&store, "p", false).await.unwrap();
    assert_eq!(stories.len(), 1);
    let s = &stories[0];
    assert_eq!(
        (
            s.accepted_at.as_deref(),
            s.epic_id.as_deref(),
            s.pending_prerequisites
        ),
        (None, None, 0)
    );
    assert!(list_epics(&store, "p").await.unwrap().is_empty());
}

// ---------- revisión: casos adicionales ----------

async fn archive(env: &Env, s: &StorySummary) {
    let current = env.summary(s).await;
    archive_story(
        &env.store,
        &ArchiveStoryInput {
            project_id: current.project_id.clone(),
            id: current.id.clone(),
            expected_revision: current.revision,
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn accepted_and_archived_story_is_still_treated_as_archived() {
    let env = Env::new().await;
    let p = env.project("Aceptada y archivada").await;
    let s = env.story(&p, "Activa").await;
    let done = env.story(&p, "Hecha y archivada").await;
    env.set_accepted(&done, true).await;
    archive(&env, &done).await;

    assert_eq!(
        env.depend(&s, &done).await.unwrap_err().code(),
        ErrorCode::Validation
    );
    assert_eq!(
        env.parent(&s, Some(&done)).await.unwrap_err().code(),
        ErrorCode::Validation
    );
    // Y como origen tampoco gana relaciones.
    assert_eq!(
        env.depend(&done, &s).await.unwrap_err().code(),
        ErrorCode::Validation
    );
}

#[tokio::test]
async fn archived_source_story_cannot_gain_relations() {
    let env = Env::new().await;
    let p = env.project("Origen archivado").await;
    let s = env.story(&p, "Archivada").await;
    let other = env.story(&p, "Otra").await;
    let epic = env.epic(&p, "Épica").await;
    archive(&env, &s).await;

    assert_eq!(
        env.join_epic(&s, Some(&epic)).await.unwrap_err().code(),
        ErrorCode::Validation
    );
    assert_eq!(
        env.parent(&s, Some(&other)).await.unwrap_err().code(),
        ErrorCode::Validation
    );
    assert_eq!(
        env.depend(&s, &other).await.unwrap_err().code(),
        ErrorCode::Validation
    );
}

#[tokio::test]
async fn failed_changes_keep_the_previous_relation() {
    let env = Env::new().await;
    let p = env.project("Rollback").await;
    let other = env.project("Otro").await;
    let epic = env.epic(&p, "Original").await;
    let foreign_epic = env.epic(&other, "Ajena").await;
    let (a, b) = (env.story(&p, "A").await, env.story(&p, "B").await);
    env.join_epic(&a, Some(&epic)).await.unwrap();
    env.parent(&b, Some(&a)).await.unwrap();

    // Épica ajena: falla y la épica anterior se conserva (el borrado previo se revierte).
    assert!(env.join_epic(&a, Some(&foreign_epic)).await.is_err());
    let rel = get_story_relations(&env.store, &p.id, &a.id).await.unwrap();
    assert_eq!(rel.epic.unwrap().id, epic.id);

    // Padre que crearía ciclo: falla y el padre anterior se conserva.
    let c = env.story(&p, "C").await;
    env.parent(&a, Some(&c)).await.unwrap();
    env.parent(&c, Some(&b)).await.unwrap_err(); // b → a → c: ciclo
    let rel = get_story_relations(&env.store, &p.id, &c.id).await.unwrap();
    assert!(rel.parent.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_opposite_dependencies_never_form_a_cycle() {
    let env = Env::new().await;
    let p = env.project("Carrera").await;
    for round in 0..10 {
        let a = env.story(&p, &format!("A{round}")).await;
        let b = env.story(&p, &format!("B{round}")).await;
        let (store_1, store_2) = (env.store.clone(), env.store.clone());
        let (ab, ba) = (dep(&a, &b.id), dep(&b, &a.id));
        let first = tokio::spawn(async move { add_story_dependency(&store_1, &ab).await });
        let second = tokio::spawn(async move { add_story_dependency(&store_2, &ba).await });
        let results = [first.await.unwrap(), second.await.unwrap()];

        let ok = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(ok, 1, "ronda {round}: exactamente una debe ganar");
        let failed = results.iter().find_map(|r| r.as_ref().err()).unwrap();
        assert_eq!(failed.code(), ErrorCode::Cycle);
    }
}

#[tokio::test]
async fn prerequisite_limit_is_enforced() {
    let env = Env::new().await;
    let p = env.project("Límite").await;
    let s = env.story(&p, "Muchas").await;
    for i in 0..50 {
        let prerequisite = env.story(&p, &format!("P{i}")).await;
        env.depend(&s, &prerequisite).await.unwrap();
    }
    let extra = env.story(&p, "Una más").await;
    let error = env.depend(&s, &extra).await.unwrap_err().to_payload(None);
    assert_eq!(
        (error.code, error.field.as_deref()),
        (ErrorCode::Validation, Some("dependsOnId"))
    );
}

#[tokio::test]
async fn story_epics_rejects_cross_project_in_the_database() {
    let env = Env::new().await;
    let mine = env.project("Mío").await;
    let other = env.project("Ajeno").await;
    let s = env.story(&mine, "Local").await;
    let foreign_epic = env.epic(&other, "Ajena").await;
    let (sid, eid, pid) = (s.id.clone(), foreign_epic.id.clone(), mine.id.clone());
    let raw = env
        .store
        .write(move |conn| {
            Box::pin(async move {
                sqlx::query(
                    "INSERT INTO story_epics (story_id, project_id, epic_id) VALUES (?, ?, ?)",
                )
                .bind(sid)
                .bind(pid)
                .bind(eid)
                .execute(&mut *conn)
                .await?;
                Ok(())
            })
        })
        .await;
    assert!(matches!(raw, Err(CoreError::Database(_))));
}

#[tokio::test]
async fn pending_prerequisites_agree_between_list_and_relations() {
    // El recuento se calcula en SQL (listas) y en Rust (panel): deben coincidir en todo estado.
    let env = Env::new().await;
    let p = env.project("Coherencia").await;
    let s = env.story(&p, "Principal").await;
    let pending = env.story(&p, "Pendiente").await;
    let accepted = env.story(&p, "Aceptada").await;
    let archived = env.story(&p, "Archivada").await;
    let accepted_archived = env.story(&p, "Aceptada y archivada").await;
    for prerequisite in [&pending, &accepted, &archived, &accepted_archived] {
        env.depend(&s, prerequisite).await.unwrap();
    }
    env.set_accepted(&accepted, true).await;
    env.set_accepted(&accepted_archived, true).await;
    archive(&env, &archived).await;
    archive(&env, &accepted_archived).await;

    let from_list = env.summary(&s).await.pending_prerequisites;
    let from_panel = get_story_relations(&env.store, &p.id, &s.id)
        .await
        .unwrap()
        .pending_prerequisites;
    assert_eq!(
        (from_list, from_panel),
        (2, 2),
        "pendiente + archivada sin aceptar"
    );
}
