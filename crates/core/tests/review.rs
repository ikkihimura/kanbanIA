//! Pruebas de HU-009 a través de la API pública del núcleo, con SQLite temporal real.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use kanbania_core::error::ErrorCode;
use kanbania_core::features::columns::{
    Column, ColumnCategory, CreateColumnInput, UpdateColumnInput, create_column, list_columns,
    update_column,
};
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::relations::{DependencyInput, add_story_dependency};
use kanbania_core::features::review::{
    AcceptStoryInput, AcceptanceEventKind, DeliverySource, DeliveryStatus, ReferenceInput,
    ReferenceKind, ReferenceStatus, ReopenStoryInput, ReturnDeliveryInput, SetBlockerInput,
    SubmitDeliveryInput, accept_story, get_story_review, list_attention, reopen_story,
    return_delivery, set_story_blocker, submit_delivery,
};
use kanbania_core::features::stories::{
    CreateStoryInput, CriterionDraft, MoveStoryInput, StoryDetail, StorySummary, UpdateStoryInput,
    create_story, get_story, list_stories, move_story, update_story,
};
use kanbania_core::features::team::{AgentFields, AvatarSpec, CreateAgentInput, create_agent};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::storage::Store;

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

struct Env {
    _data: tempfile::TempDir,
    store: Store,
    project: Project,
}

impl Env {
    async fn new() -> Self {
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).await.unwrap();
        let project = create_project(
            &store,
            "ui",
            &CreateProjectInput {
                request_id: rid(),
                name: "Revisión".into(),
            },
        )
        .await
        .unwrap();
        Self {
            _data: data,
            store,
            project,
        }
    }

    async fn column(&self, category: ColumnCategory) -> Column {
        list_columns(&self.store, &self.project.id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.category == category)
            .unwrap()
    }

    /// Historia con criterios (CA-1..CA-n).
    async fn story(&self, title: &str, criteria: &[&str]) -> StoryDetail {
        let s = create_story(
            &self.store,
            "ui",
            &CreateStoryInput {
                request_id: rid(),
                project_id: self.project.id.clone(),
                title: title.into(),
                column_id: None,
                priority: None,
            },
        )
        .await
        .unwrap();
        let detail = get_story(&self.store, &self.project.id, &s.id)
            .await
            .unwrap();
        update_story(
            &self.store,
            &UpdateStoryInput {
                project_id: self.project.id.clone(),
                id: s.id.clone(),
                expected_revision: detail.revision,
                title: detail.title.clone(),
                description: String::new(),
                priority: detail.priority,
                criteria: criteria
                    .iter()
                    .map(|t| CriterionDraft {
                        id: None,
                        text: (*t).into(),
                    })
                    .collect(),
            },
        )
        .await
        .unwrap()
    }

    async fn detail(&self, id: &str) -> StoryDetail {
        get_story(&self.store, &self.project.id, id).await.unwrap()
    }

    async fn summary(&self, id: &str) -> StorySummary {
        list_stories(&self.store, &self.project.id, true)
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap()
    }

    fn deliver(&self, story: &StoryDetail, summary: &str, to_review: bool) -> SubmitDeliveryInput {
        SubmitDeliveryInput {
            request_id: rid(),
            project_id: self.project.id.clone(),
            story_id: story.id.clone(),
            summary: summary.into(),
            references: Vec::new(),
            send_to_review: to_review,
        }
    }

    async fn accept_all(
        &self,
        actor: &Actor,
        id: &str,
    ) -> Result<StorySummary, kanbania_core::error::CoreError> {
        let d = self.detail(id).await;
        accept_story(
            &self.store,
            actor,
            &AcceptStoryInput {
                project_id: self.project.id.clone(),
                story_id: id.into(),
                expected_revision: d.revision,
                satisfied_criteria: d.criteria.iter().map(|c| c.id.clone()).collect(),
                column_id: None,
                to_index: None,
            },
        )
        .await
    }

    async fn agent(&self) -> Actor {
        let agent = create_agent(
            &self.store,
            "ui",
            &CreateAgentInput {
                request_id: rid(),
                fields: AgentFields {
                    name: "Robota".into(),
                    role: String::new(),
                    provider: String::new(),
                    model: String::new(),
                    capabilities: Vec::new(),
                    avatar: AvatarSpec::DEFAULT_AGENT,
                },
            },
        )
        .await
        .unwrap();
        Actor::Agent { agent_id: agent.id }
    }
}

// ---------- CA-1: entregas manuales o reportadas; enviar a revisión sin marcar hecho ----------

#[tokio::test]
async fn manual_delivery_goes_to_review_without_marking_done() {
    let env = Env::new().await;
    let story = env.story("Informe", &["Descarga CSV"]).await;
    let review = env.column(ColumnCategory::Review).await;
    let dir = tempfile::tempdir().unwrap();
    let report = dir.path().join("informe.csv");
    std::fs::write(&report, "a,b").unwrap();

    let mut input = env.deliver(&story, "  Exporta el informe en CSV  ", true);
    input.references = vec![
        ReferenceInput {
            kind: ReferenceKind::Url,
            value: "https://github.com/o/r/pull/7".into(),
        },
        ReferenceInput {
            kind: ReferenceKind::Path,
            value: report.to_str().unwrap().into(),
        },
        ReferenceInput {
            kind: ReferenceKind::Path,
            value: dir.path().join("borrado.txt").to_str().unwrap().into(),
        },
    ];
    let delivery = submit_delivery(&env.store, &Actor::Human, &input)
        .await
        .unwrap();

    assert_eq!(
        (delivery.number, delivery.source),
        (1, DeliverySource::Manual)
    );
    assert_eq!(delivery.summary, "Exporta el informe en CSV");
    assert_eq!(delivery.status, DeliveryStatus::Pending);
    let statuses: Vec<_> = delivery.references.iter().map(|r| r.status).collect();
    assert_eq!(
        statuses,
        [
            ReferenceStatus::Unverified,
            ReferenceStatus::Accessible,
            ReferenceStatus::Missing
        ]
    );

    let after = env.summary(&story.id).await;
    assert_eq!(after.column_id, review.id, "enviada a revisión");
    assert_eq!(after.accepted_at, None, "sin marcar como hecha");

    // Reintento idempotente: misma entrega, sin duplicar.
    let again = submit_delivery(&env.store, &Actor::Human, &input)
        .await
        .unwrap();
    assert_eq!(again.id, delivery.id);
    assert_eq!(
        get_story_review(&env.store, &env.project.id, &story.id)
            .await
            .unwrap()
            .deliveries
            .len(),
        1
    );
}

#[tokio::test]
async fn agent_reports_delivery_and_blocker_but_cannot_decide() {
    let env = Env::new().await;
    let story = env.story("Integración", &["Pasa la prueba"]).await;
    let agent = env.agent().await;

    let delivery = submit_delivery(
        &env.store,
        &agent,
        &env.deliver(&story, "Hecho por el agente", false),
    )
    .await
    .unwrap();
    assert_eq!(delivery.source, DeliverySource::Reported);
    assert_eq!(delivery.agent_name.as_deref(), Some("Robota"));

    set_story_blocker(
        &env.store,
        &agent,
        &SetBlockerInput {
            project_id: env.project.id.clone(),
            story_id: story.id.clone(),
            reason: Some("Falta acceso a la API".into()),
        },
    )
    .await
    .unwrap();
    let review = get_story_review(&env.store, &env.project.id, &story.id)
        .await
        .unwrap();
    assert_eq!(review.blocker.unwrap().source, DeliverySource::Reported);

    // Un agente no acepta, no reabre ni devuelve entregas.
    let accept = env
        .accept_all(&agent, &story.id)
        .await
        .unwrap_err()
        .to_payload(None);
    assert_eq!(accept.code, ErrorCode::Forbidden);
    assert!(!accept.recoverable);
    let returned = return_delivery(
        &env.store,
        &agent,
        &ReturnDeliveryInput {
            project_id: env.project.id.clone(),
            delivery_id: delivery.id.clone(),
            reason: "No".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(returned.code(), ErrorCode::Forbidden);
    let reopen = reopen_story(
        &env.store,
        &agent,
        &ReopenStoryInput {
            project_id: env.project.id.clone(),
            story_id: story.id.clone(),
            expected_revision: 1,
            reason: "x".into(),
            column_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(reopen.code(), ErrorCode::Forbidden);
    assert_eq!(env.summary(&story.id).await.accepted_at, None);
}

#[tokio::test]
async fn invalid_deliveries_are_rejected() {
    let env = Env::new().await;
    let story = env.story("Validar", &["x"]).await;

    let empty = submit_delivery(&env.store, &Actor::Human, &env.deliver(&story, "  ", false))
        .await
        .unwrap_err();
    assert_eq!(empty.to_payload(None).field.as_deref(), Some("summary"));

    let mut bad_url = env.deliver(&story, "ok", false);
    bad_url.references = vec![ReferenceInput {
        kind: ReferenceKind::Url,
        value: "javascript:alert(1)".into(),
    }];
    let error = submit_delivery(&env.store, &Actor::Human, &bad_url)
        .await
        .unwrap_err();
    assert_eq!(error.to_payload(None).field.as_deref(), Some("references"));

    let ghost = Actor::Agent {
        agent_id: "no-existe".into(),
    };
    assert_eq!(
        submit_delivery(&env.store, &ghost, &env.deliver(&story, "ok", false))
            .await
            .unwrap_err()
            .code(),
        ErrorCode::NotFound
    );
}

// ---------- CA-2: aceptar (persona + criterios), reabrir con historial ----------

#[tokio::test]
async fn acceptance_requires_all_criteria_and_at_least_one() {
    let env = Env::new().await;
    let none = env.story("Sin criterios", &[]).await;
    let error = env
        .accept_all(&Actor::Human, &none.id)
        .await
        .unwrap_err()
        .to_payload(None);
    assert_eq!(error.field.as_deref(), Some("satisfiedCriteria"));
    assert!(error.message.contains("al menos un criterio"));

    let story = env.story("Con criterios", &["uno", "dos"]).await;
    let partial = accept_story(
        &env.store,
        &Actor::Human,
        &AcceptStoryInput {
            project_id: env.project.id.clone(),
            story_id: story.id.clone(),
            expected_revision: story.revision,
            satisfied_criteria: vec![story.criteria[0].id.clone()],
            column_id: None,
            to_index: None,
        },
    )
    .await
    .unwrap_err()
    .to_payload(None);
    assert!(partial.message.contains("CA-2"), "{}", partial.message);
    assert_eq!(env.summary(&story.id).await.accepted_at, None);
}

#[tokio::test]
async fn accept_moves_to_done_records_history_and_resolves_pending_deliveries() {
    let env = Env::new().await;
    let story = env.story("Aceptable", &["Funciona", "Documentado"]).await;
    submit_delivery(
        &env.store,
        &Actor::Human,
        &env.deliver(&story, "Primera", true),
    )
    .await
    .unwrap();
    let done = env.column(ColumnCategory::Done).await;

    let accepted = env.accept_all(&Actor::Human, &story.id).await.unwrap();
    assert!(accepted.accepted_at.is_some());
    assert_eq!(accepted.column_id, done.id);

    let review = get_story_review(&env.store, &env.project.id, &story.id)
        .await
        .unwrap();
    assert_eq!(review.deliveries[0].status, DeliveryStatus::Accepted);
    assert_eq!(review.history.len(), 1);
    let event = &review.history[0];
    assert_eq!(event.kind, AcceptanceEventKind::Accepted);
    let snapshot: Vec<_> = event
        .criteria
        .iter()
        .map(|c| (c.number, c.text.as_str()))
        .collect();
    assert_eq!(snapshot, [(1, "Funciona"), (2, "Documentado")]);

    // Ya aceptada: no se acepta dos veces ni se le entrega de nuevo.
    assert_eq!(
        env.accept_all(&Actor::Human, &story.id)
            .await
            .unwrap_err()
            .code(),
        ErrorCode::Validation
    );
    let redeliver = submit_delivery(
        &env.store,
        &Actor::Human,
        &env.deliver(&story, "otra", false),
    )
    .await
    .unwrap_err();
    assert_eq!(redeliver.code(), ErrorCode::Validation);
}

#[tokio::test]
async fn reopen_keeps_history_and_leaves_done() {
    let env = Env::new().await;
    let story = env.story("Reabrir", &["ok"]).await;
    let accepted = env.accept_all(&Actor::Human, &story.id).await.unwrap();
    let review_column = env.column(ColumnCategory::Review).await;

    let empty_reason = reopen_story(
        &env.store,
        &Actor::Human,
        &ReopenStoryInput {
            project_id: env.project.id.clone(),
            story_id: story.id.clone(),
            expected_revision: accepted.revision,
            reason: "  ".into(),
            column_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(
        empty_reason.to_payload(None).field.as_deref(),
        Some("reason")
    );

    let reopened = reopen_story(
        &env.store,
        &Actor::Human,
        &ReopenStoryInput {
            project_id: env.project.id.clone(),
            story_id: story.id.clone(),
            expected_revision: accepted.revision,
            reason: "Falla en producción".into(),
            column_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(reopened.accepted_at, None);
    assert_eq!(
        reopened.column_id, review_column.id,
        "por defecto a revisión"
    );

    let history = get_story_review(&env.store, &env.project.id, &story.id)
        .await
        .unwrap()
        .history;
    let kinds: Vec<_> = history.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [AcceptanceEventKind::Reopened, AcceptanceEventKind::Accepted]
    );
    assert_eq!(history[0].reason.as_deref(), Some("Falla en producción"));

    // Se puede volver a aceptar; el historial crece.
    env.accept_all(&Actor::Human, &story.id).await.unwrap();
    let history = get_story_review(&env.store, &env.project.id, &story.id)
        .await
        .unwrap()
        .history;
    assert_eq!(history.len(), 3);
}

#[tokio::test]
async fn accepted_cards_only_leave_done_by_reopening() {
    let env = Env::new().await;
    let story = env.story("En hecho", &["ok"]).await;
    let accepted = env.accept_all(&Actor::Human, &story.id).await.unwrap();
    let active = env.column(ColumnCategory::Active).await;

    let moved = move_story(
        &env.store,
        &MoveStoryInput {
            project_id: env.project.id.clone(),
            id: story.id.clone(),
            expected_revision: accepted.revision,
            to_column_id: active.id.clone(),
            to_index: 0,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(moved.code(), ErrorCode::Validation);
    assert!(env.summary(&story.id).await.accepted_at.is_some());
}

#[tokio::test]
async fn returned_delivery_keeps_reason_and_story_place() {
    let env = Env::new().await;
    let story = env.story("Devolver", &["ok"]).await;
    let delivery = submit_delivery(&env.store, &Actor::Human, &env.deliver(&story, "v1", true))
        .await
        .unwrap();
    let before = env.summary(&story.id).await;

    return_delivery(
        &env.store,
        &Actor::Human,
        &ReturnDeliveryInput {
            project_id: env.project.id.clone(),
            delivery_id: delivery.id.clone(),
            reason: "Faltan pruebas".into(),
        },
    )
    .await
    .unwrap();
    let review = get_story_review(&env.store, &env.project.id, &story.id)
        .await
        .unwrap();
    assert_eq!(review.deliveries[0].status, DeliveryStatus::Returned);
    assert_eq!(
        review.deliveries[0].return_reason.as_deref(),
        Some("Faltan pruebas")
    );
    assert_eq!(env.summary(&story.id).await.column_id, before.column_id);

    // Ya decidida: no se devuelve otra vez.
    let again = return_delivery(
        &env.store,
        &Actor::Human,
        &ReturnDeliveryInput {
            project_id: env.project.id.clone(),
            delivery_id: delivery.id,
            reason: "x".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(again.code(), ErrorCode::Validation);
}

// ---------- CA-3: bandeja sin duplicados y coherente con la tarjeta ----------

#[tokio::test]
async fn attention_lists_each_story_once_with_all_reasons() {
    let env = Env::new().await;
    let blocked = env.story("Bloqueada y entregada", &["ok"]).await;
    let waiting = env.story("Depende", &["ok"]).await;
    let prereq = env.story("Prerequisito", &["ok"]).await;
    let quiet = env.story("Tranquila", &["ok"]).await;
    let backlog_dep = env.story("En backlog con dependencia", &["ok"]).await;

    set_story_blocker(
        &env.store,
        &Actor::Human,
        &SetBlockerInput {
            project_id: env.project.id.clone(),
            story_id: blocked.id.clone(),
            reason: Some("Esperando diseño".into()),
        },
    )
    .await
    .unwrap();
    submit_delivery(&env.store, &Actor::Human, &env.deliver(&blocked, "a", true))
        .await
        .unwrap();
    submit_delivery(
        &env.store,
        &Actor::Human,
        &env.deliver(&blocked, "b", false),
    )
    .await
    .unwrap();

    // `waiting` está en curso con un prerequisito sin aceptar; `backlog_dep` sigue en backlog.
    let active = env.column(ColumnCategory::Active).await;
    let w = env.summary(&waiting.id).await;
    move_story(
        &env.store,
        &MoveStoryInput {
            project_id: env.project.id.clone(),
            id: w.id.clone(),
            expected_revision: w.revision,
            to_column_id: active.id.clone(),
            to_index: 0,
        },
    )
    .await
    .unwrap();
    for s in [&waiting, &backlog_dep] {
        add_story_dependency(
            &env.store,
            &DependencyInput {
                project_id: env.project.id.clone(),
                story_id: s.id.clone(),
                depends_on_id: prereq.id.clone(),
            },
        )
        .await
        .unwrap();
    }

    let items = list_attention(&env.store, &env.project.id).await.unwrap();
    let ids: Vec<_> = items.iter().map(|i| i.story_id.as_str()).collect();
    assert_eq!(
        ids.len(),
        2,
        "sin duplicados ni historias tranquilas: {ids:?}"
    );
    let b = items.iter().find(|i| i.story_id == blocked.id).unwrap();
    assert_eq!(
        (b.blocker_reason.as_deref(), b.pending_deliveries),
        (Some("Esperando diseño"), 2)
    );
    let w = items.iter().find(|i| i.story_id == waiting.id).unwrap();
    assert_eq!(w.pending_prerequisites, 1);
    assert!(!ids.contains(&quiet.id.as_str()));
    assert!(
        !ids.contains(&backlog_dep.id.as_str()),
        "en backlog las dependencias no alarman"
    );

    // Aceptar el prerequisito y la bloqueada (quitando su bloqueo) vacía la bandeja: coherente.
    env.accept_all(&Actor::Human, &prereq.id).await.unwrap();
    set_story_blocker(
        &env.store,
        &Actor::Human,
        &SetBlockerInput {
            project_id: env.project.id.clone(),
            story_id: blocked.id.clone(),
            reason: None,
        },
    )
    .await
    .unwrap();
    env.accept_all(&Actor::Human, &blocked.id).await.unwrap();
    assert!(
        list_attention(&env.store, &env.project.id)
            .await
            .unwrap()
            .is_empty()
    );

    // Reabrir el prerequisito devuelve a `waiting` a la bandeja.
    let p = env.summary(&prereq.id).await;
    reopen_story(
        &env.store,
        &Actor::Human,
        &ReopenStoryInput {
            project_id: env.project.id.clone(),
            story_id: p.id.clone(),
            expected_revision: p.revision,
            reason: "Regresión".into(),
            column_id: None,
        },
    )
    .await
    .unwrap();
    let ids: Vec<_> = list_attention(&env.store, &env.project.id)
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.story_id)
        .collect();
    assert!(ids.contains(&waiting.id));
}

#[tokio::test]
async fn accept_without_done_column_explains_the_problem() {
    let env = Env::new().await;
    let story = env.story("Sin hecho", &["ok"]).await;
    let done = env.column(ColumnCategory::Done).await;
    kanbania_core::features::columns::delete_column(
        &env.store,
        &kanbania_core::features::columns::DeleteColumnInput {
            project_id: env.project.id.clone(),
            id: done.id,
            expected_revision: done.revision,
            destination_column_id: None,
        },
    )
    .await
    .unwrap();

    let error = env
        .accept_all(&Actor::Human, &story.id)
        .await
        .unwrap_err()
        .to_payload(None);
    assert!(
        error.message.contains("columna de tipo «hecho»"),
        "{}",
        error.message
    );
}

// ---------- revisión: casos adicionales ----------

fn blocker(env: &Env, story: &StoryDetail, reason: Option<&str>) -> SetBlockerInput {
    SetBlockerInput {
        project_id: env.project.id.clone(),
        story_id: story.id.clone(),
        reason: reason.map(str::to_owned),
    }
}

#[tokio::test]
async fn accepting_clears_the_blocker_so_it_does_not_return_on_reopen() {
    let env = Env::new().await;
    let story = env.story("Bloqueada", &["ok"]).await;
    set_story_blocker(
        &env.store,
        &Actor::Human,
        &blocker(&env, &story, Some("Esperando")),
    )
    .await
    .unwrap();

    let accepted = env.accept_all(&Actor::Human, &story.id).await.unwrap();
    reopen_story(
        &env.store,
        &Actor::Human,
        &ReopenStoryInput {
            project_id: env.project.id.clone(),
            story_id: story.id.clone(),
            expected_revision: accepted.revision,
            reason: "Regresión".into(),
            column_id: None,
        },
    )
    .await
    .unwrap();

    let review = get_story_review(&env.store, &env.project.id, &story.id)
        .await
        .unwrap();
    assert!(review.blocker.is_none());
    let attention = list_attention(&env.store, &env.project.id).await.unwrap();
    assert!(attention.iter().all(|i| i.blocker_reason.is_none()));
}

#[tokio::test]
async fn agents_cannot_hide_human_blockers_but_manage_their_own() {
    let env = Env::new().await;
    let story = env.story("Bloqueos", &["ok"]).await;
    let agent = env.agent().await;

    set_story_blocker(
        &env.store,
        &Actor::Human,
        &blocker(&env, &story, Some("Decisión humana")),
    )
    .await
    .unwrap();
    for change in [None, Some("Otro motivo")] {
        let error = set_story_blocker(&env.store, &agent, &blocker(&env, &story, change))
            .await
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Forbidden);
    }
    let review = get_story_review(&env.store, &env.project.id, &story.id)
        .await
        .unwrap();
    assert_eq!(review.blocker.unwrap().reason, "Decisión humana");

    // La persona lo quita; el agente reporta el suyo y puede quitarlo.
    set_story_blocker(&env.store, &Actor::Human, &blocker(&env, &story, None))
        .await
        .unwrap();
    set_story_blocker(
        &env.store,
        &agent,
        &blocker(&env, &story, Some("Sin acceso")),
    )
    .await
    .unwrap();
    set_story_blocker(&env.store, &agent, &blocker(&env, &story, None))
        .await
        .unwrap();
    assert!(
        get_story_review(&env.store, &env.project.id, &story.id)
            .await
            .unwrap()
            .blocker
            .is_none()
    );
}

#[tokio::test]
async fn agent_cannot_reopen_an_accepted_story() {
    let env = Env::new().await;
    let story = env.story("Aceptada", &["ok"]).await;
    let agent = env.agent().await;
    let accepted = env.accept_all(&Actor::Human, &story.id).await.unwrap();

    let error = reopen_story(
        &env.store,
        &agent,
        &ReopenStoryInput {
            project_id: env.project.id.clone(),
            story_id: story.id.clone(),
            expected_revision: accepted.revision,
            reason: "Quiero rehacerla".into(),
            column_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Forbidden);
    assert!(env.summary(&story.id).await.accepted_at.is_some());
}

#[tokio::test]
async fn accepted_cards_respect_done_on_column_delete_and_restore() {
    use kanbania_core::features::columns::{DeleteColumnInput, delete_column};
    use kanbania_core::features::stories::{
        ArchiveStoryInput, RestoreStoryInput, archive_story, restore_story,
    };

    let env = Env::new().await;
    let story = env.story("En hecho", &["ok"]).await;
    env.accept_all(&Actor::Human, &story.id).await.unwrap();
    let done = env.column(ColumnCategory::Done).await;
    let active = env.column(ColumnCategory::Active).await;

    let error = delete_column(
        &env.store,
        &DeleteColumnInput {
            project_id: env.project.id.clone(),
            id: done.id.clone(),
            expected_revision: done.revision,
            destination_column_id: Some(active.id.clone()),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_payload(None).field.as_deref(),
        Some("destinationColumnId")
    );
    assert!(env.summary(&story.id).await.accepted_at.is_some());

    // Archivada y aceptada: al restaurar vuelve a «hecho», no a otra columna.
    let s = env.summary(&story.id).await;
    archive_story(
        &env.store,
        &ArchiveStoryInput {
            project_id: env.project.id.clone(),
            id: s.id.clone(),
            expected_revision: s.revision,
        },
    )
    .await
    .unwrap();
    let s = env.summary(&story.id).await;
    let to_active = restore_story(
        &env.store,
        &RestoreStoryInput {
            project_id: env.project.id.clone(),
            id: s.id.clone(),
            expected_revision: s.revision,
            column_id: Some(active.id.clone()),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(to_active.code(), ErrorCode::Validation);
    let restored = restore_story(
        &env.store,
        &RestoreStoryInput {
            project_id: env.project.id.clone(),
            id: s.id.clone(),
            expected_revision: s.revision,
            column_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(restored.column_id, done.id);
}

#[tokio::test]
async fn attention_is_isolated_per_project_and_ignores_plain_edits_in_order() {
    let env = Env::new().await;
    let other = create_project(
        &env.store,
        "ui",
        &CreateProjectInput {
            request_id: rid(),
            name: "Otro".into(),
        },
    )
    .await
    .unwrap();
    let first = env.story("Primera bloqueada", &["ok"]).await;
    let second = env.story("Segunda bloqueada", &["ok"]).await;
    set_story_blocker(&env.store, &Actor::Human, &blocker(&env, &first, Some("a")))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    set_story_blocker(
        &env.store,
        &Actor::Human,
        &blocker(&env, &second, Some("b")),
    )
    .await
    .unwrap();

    // Editar la primera (no es un motivo de atención) no la adelanta.
    let detail = env.detail(&first.id).await;
    update_story(
        &env.store,
        &UpdateStoryInput {
            project_id: env.project.id.clone(),
            id: first.id.clone(),
            expected_revision: detail.revision,
            title: "Primera (editada)".into(),
            description: String::new(),
            priority: detail.priority,
            criteria: detail
                .criteria
                .iter()
                .map(|c| CriterionDraft {
                    id: Some(c.id.clone()),
                    text: c.text.clone(),
                })
                .collect(),
        },
    )
    .await
    .unwrap();

    let order: Vec<_> = list_attention(&env.store, &env.project.id)
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.story_id)
        .collect();
    assert_eq!(order, [second.id.clone(), first.id.clone()]);
    assert!(
        list_attention(&env.store, &other.id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn invisible_formatting_and_network_paths_are_rejected_and_agents_get_no_disk_status() {
    let env = Env::new().await;
    let story = env.story("Texto", &["ok"]).await;

    let disguised = submit_delivery(
        &env.store,
        &Actor::Human,
        &env.deliver(&story, "Todo bien \u{202E}lim", false),
    )
    .await
    .unwrap_err();
    assert_eq!(disguised.to_payload(None).field.as_deref(), Some("summary"));

    let mut network = env.deliver(&story, "ok", false);
    network.references = vec![ReferenceInput {
        kind: ReferenceKind::Path,
        value: "//servidor/compartido/informe".into(),
    }];
    let error = submit_delivery(&env.store, &Actor::Human, &network)
        .await
        .unwrap_err();
    assert_eq!(error.to_payload(None).field.as_deref(), Some("references"));

    // El agente no recibe si una ruta existe: siempre «no verificada».
    let dir = tempfile::tempdir().unwrap();
    let agent = env.agent().await;
    let mut probe = env.deliver(&story, "sondeo", false);
    probe.references = vec![ReferenceInput {
        kind: ReferenceKind::Path,
        value: dir.path().to_str().unwrap().into(),
    }];
    let delivery = submit_delivery(&env.store, &agent, &probe).await.unwrap();
    assert_eq!(delivery.references[0].status, ReferenceStatus::Unverified);
    // La persona sí lo ve al revisar.
    let review = get_story_review(&env.store, &env.project.id, &story.id)
        .await
        .unwrap();
    assert_eq!(
        review.deliveries[0].references[0].status,
        ReferenceStatus::Accessible
    );
}

/// Soltar en una segunda columna «hecho» acepta y coloca en una sola transacción, con el WIP
/// de esa columna (no el de la primera).
#[tokio::test]
async fn accept_into_chosen_done_column_and_index_is_atomic() {
    let env = Env::new().await;
    let first_done = env.column(ColumnCategory::Done).await;
    let archive = create_column(
        &env.store,
        "ui",
        &CreateColumnInput {
            request_id: rid(),
            project_id: env.project.id.clone(),
            name: "Archivo".into(),
            category: ColumnCategory::Done,
            wip_limit: None,
        },
    )
    .await
    .unwrap();

    // Llenar la primera «hecho» (WIP 1).
    let a = env.story("A", &["ok"]).await;
    env.accept_all(&Actor::Human, &a.id).await.unwrap();
    update_column(
        &env.store,
        &UpdateColumnInput {
            project_id: env.project.id.clone(),
            id: first_done.id.clone(),
            expected_revision: first_done.revision,
            name: first_done.name.clone(),
            category: ColumnCategory::Done,
            wip_limit: Some(1),
        },
    )
    .await
    .unwrap();

    let accept_into =
        |story: StoryDetail, column_id: &str, to_index: Option<i64>| AcceptStoryInput {
            project_id: env.project.id.clone(),
            story_id: story.id.clone(),
            expected_revision: story.revision,
            satisfied_criteria: story.criteria.iter().map(|c| c.id.clone()).collect(),
            column_id: Some(column_id.into()),
            to_index,
        };

    // Por defecto iría a la primera (llena): WIP.
    let b = env.story("B", &["ok"]).await;
    let err = env.accept_all(&Actor::Human, &b.id).await.unwrap_err();
    assert_eq!(err.code(), ErrorCode::WipLimitExceeded);
    let b_now = env.summary(&b.id).await;
    assert!(
        b_now.accepted_at.is_none(),
        "el fallo no deja nada a medias"
    );

    // A la segunda «hecho», al final.
    let accepted = accept_story(
        &env.store,
        &Actor::Human,
        &accept_into(env.detail(&b.id).await, &archive.id, None),
    )
    .await
    .unwrap();
    assert_eq!(accepted.column_id, archive.id);
    assert_eq!(accepted.position, Some(0));

    // Otra, delante de B (índice 0); un índice enorme se acotaría al final.
    let c = env.story("C", &["ok"]).await;
    let accepted = accept_story(
        &env.store,
        &Actor::Human,
        &accept_into(c, &archive.id, Some(0)),
    )
    .await
    .unwrap();
    assert_eq!(accepted.position, Some(0));
    assert_eq!(env.summary(&b.id).await.position, Some(1));

    // Una columna que no es «hecho» se rechaza sin aceptar.
    let d = env.story("D", &["ok"]).await;
    let review = env.column(ColumnCategory::Review).await;
    let err = accept_story(
        &env.store,
        &Actor::Human,
        &accept_into(d.clone(), &review.id, None),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Validation);
    let err = accept_story(
        &env.store,
        &Actor::Human,
        &accept_into(d.clone(), &archive.id, Some(-1)),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Validation);
    assert!(env.summary(&d.id).await.accepted_at.is_none());
}
