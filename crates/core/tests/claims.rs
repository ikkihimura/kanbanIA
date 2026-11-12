//! Reclamos de historias (HU-014) con SQLite temporal real y reloj controlado.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use kanbania_core::error::{CoreError, ErrorCode};
use kanbania_core::features::access::{
    AgentSession, CreateAgentCredentialInput, authenticate_agent, create_agent_credential,
};
use kanbania_core::features::agent_work::{
    ReadyQuery, StoryKey, get_story_context, list_ready_stories,
};
use kanbania_core::features::claims::{
    CLAIM_DURATION_MINUTES, ClaimStoryInput, MAX_OPEN_CLAIMS, ReleaseClaimInput, RenewClaimInput,
    StoryClaim, claim_story, list_active_claims, release_claim, renew_claim,
};
use kanbania_core::features::columns::{Column, ColumnCategory, list_columns};
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::relations::{
    DependencyInput, SetStoryAgentInput, add_story_dependency, set_story_agent,
};
use kanbania_core::features::review::{SetBlockerInput, set_story_blocker};
use kanbania_core::features::stories::{
    ArchiveStoryInput, CreateStoryInput, Priority, StorySummary, archive_story, create_story,
    list_stories,
};
use kanbania_core::features::team::{
    Agent, AgentFields, AvatarSpec, CreateAgentInput, create_agent,
};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::clock::ManualClock;
use kanbania_core::platform::storage::{BackupKind, Store, backup_path, restore_backup};

const UI: &str = "ui";
const MINUTE: Duration = Duration::from_secs(60);
/// 2026-11-15T12:00:00Z: hora fija a mitad de mes (`iso_ms` solo compara dentro del mes).
const START_MS: i64 = 1_794_744_000_000;

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

struct Board {
    _data: tempfile::TempDir,
    store: Store,
    clock: Arc<ManualClock>,
    project: Project,
    columns: Vec<Column>,
    me: Agent,
    other: Agent,
    mine: AgentSession,
    theirs: AgentSession,
}

impl Board {
    async fn new() -> Self {
        let data = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::new(START_MS));
        let store = Store::open(data.path())
            .await
            .unwrap()
            .with_clock(clock.clone());
        let project = project(&store, "Mío").await;
        let columns = list_columns(&store, &project.id).await.unwrap();
        let me = agent(&store, "Yo-bot").await;
        let other = agent(&store, "Otro-bot").await;
        let mine = session(&store, &project, &me).await;
        let theirs = session(&store, &project, &other).await;
        Self {
            _data: data,
            store,
            clock,
            project,
            columns,
            me,
            other,
            mine,
            theirs,
        }
    }

    fn column(&self, category: ColumnCategory) -> String {
        self.columns
            .iter()
            .find(|c| c.category == category)
            .unwrap()
            .id
            .clone()
    }

    async fn story(&self, title: &str, category: ColumnCategory) -> StorySummary {
        create_story(
            &self.store,
            UI,
            &CreateStoryInput {
                request_id: rid(),
                project_id: self.project.id.clone(),
                title: title.into(),
                column_id: Some(self.column(category)),
                priority: Some(Priority::Medium),
            },
        )
        .await
        .unwrap()
    }

    async fn ready(&self, title: &str) -> StorySummary {
        self.story(title, ColumnCategory::Ready).await
    }

    async fn assign(&self, s: &StorySummary, agent: Option<&Agent>) {
        set_story_agent(
            &self.store,
            &SetStoryAgentInput {
                project_id: self.project.id.clone(),
                story_id: s.id.clone(),
                agent_id: agent.map(|a| a.id.clone()),
            },
        )
        .await
        .unwrap();
    }

    async fn assigned_to(&self, s: &StorySummary) -> Option<String> {
        list_stories(&self.store, &self.project.id, false)
            .await
            .unwrap()
            .into_iter()
            .find(|x| x.id == s.id)
            .unwrap()
            .agent_id
    }

    async fn open_claims(&self, s: &StorySummary) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM story_claims WHERE story_id = ? AND ended_at_ms IS NULL",
        )
        .bind(&s.id)
        .fetch_one(self.store.reader())
        .await
        .unwrap()
    }

    async fn ready_ids(&self, session: &AgentSession) -> Vec<String> {
        list_ready_stories(&self.store, session, &ReadyQuery::default())
            .await
            .unwrap()
            .stories
            .into_iter()
            .map(|s| s.id)
            .collect()
    }

    async fn release(&self, claim: &StoryClaim) -> Result<(), CoreError> {
        release_claim(
            &self.store,
            &Actor::Human,
            &ReleaseClaimInput {
                project_id: self.project.id.clone(),
                claim_id: claim.claim_id.clone(),
            },
        )
        .await
    }
}

fn by_id(s: &StorySummary) -> ClaimStoryInput {
    ClaimStoryInput {
        request_id: rid(),
        story_id: Some(s.id.clone()),
        number: None,
    }
}

fn renewal(claim: &StoryClaim) -> RenewClaimInput {
    RenewClaimInput {
        request_id: rid(),
        claim_id: claim.claim_id.clone(),
    }
}

fn code<T: std::fmt::Debug>(result: Result<T, CoreError>) -> ErrorCode {
    result.unwrap_err().code()
}

/// Milisegundos entre dos fechas ISO del esquema (`…T10:30:00.000Z`).
fn ms_between(from: &str, to: &str) -> i64 {
    iso_ms(to) - iso_ms(from)
}

fn iso_ms(iso: &str) -> i64 {
    // hh:mm:ss.fff del mismo día o del siguiente: suficiente para diferencias < 24 h.
    let (date, time) = iso.trim_end_matches('Z').split_once('T').unwrap();
    let day: i64 = date.rsplit('-').next().unwrap().parse().unwrap();
    let mut parts = time.split(':');
    let h: i64 = parts.next().unwrap().parse().unwrap();
    let m: i64 = parts.next().unwrap().parse().unwrap();
    let s: f64 = parts.next().unwrap().parse().unwrap();
    #[allow(clippy::cast_possible_truncation)]
    let s_ms = (s * 1000.0).round() as i64;
    ((day * 24 + h) * 60 + m) * 60_000 + s_ms
}

// ---------- CA-1: reclamar en una transacción ----------

#[tokio::test]
async fn claim_returns_owner_and_expiry_and_assigns_the_story() {
    let b = Board::new().await;
    let s = b.ready("Login").await;

    let claim = claim_story(&b.store, &b.mine, &by_id(&s)).await.unwrap();
    assert_eq!(claim.story_id, s.id);
    assert_eq!(claim.story_number, s.number);
    assert_eq!(claim.owner.agent_id, b.me.id);
    assert_eq!(claim.owner.agent_name, "Yo-bot");
    assert!(claim.renewed_at.is_none());
    assert_eq!(
        ms_between(&claim.claimed_at, &claim.expires_at),
        CLAIM_DURATION_MINUTES * 60_000
    );
    // Decisión del propietario: reclamar asigna la historia.
    assert_eq!(b.assigned_to(&s).await, Some(b.me.id.clone()));

    // Para mí sigue lista (puedo releer el contexto); para el otro agente desaparece.
    assert!(b.ready_ids(&b.mine).await.contains(&s.id));
    assert!(!b.ready_ids(&b.theirs).await.contains(&s.id));
    let context = get_story_context(&b.store, &b.mine, &StoryKey::Id(s.id.clone()))
        .await
        .unwrap();
    let yours = context.your_claim.unwrap();
    assert_eq!(yours.claim_id, claim.claim_id);
    assert_eq!(yours.expires_at, claim.expires_at);
    assert!(!context.claimed_by_other);

    // Volver a reclamarla (otra petición) devuelve el mismo reclamo, sin crear otro.
    let again = claim_story(
        &b.store,
        &b.mine,
        &ClaimStoryInput {
            request_id: rid(),
            story_id: None,
            number: Some(s.number),
        },
    )
    .await
    .unwrap();
    assert_eq!(again, claim);
    assert_eq!(b.open_claims(&s).await, 1);

    // El otro agente recibe `claimed`, y la UI lo ve en la lista de reclamos vigentes.
    assert_eq!(
        code(claim_story(&b.store, &b.theirs, &by_id(&s)).await),
        ErrorCode::Claimed
    );
    let active = list_active_claims(&b.store, &b.project.id).await.unwrap();
    assert_eq!(active, vec![claim]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_claims_by_two_agents_have_exactly_one_winner() {
    let b = Board::new().await;
    for round in 0..10 {
        let s = b.ready(&format!("Carrera {round}")).await;
        let (store_a, store_b) = (b.store.clone(), b.store.clone());
        let (mine, theirs) = (b.mine.clone(), b.theirs.clone());
        let (input_a, input_b) = (by_id(&s), by_id(&s));
        let a = tokio::spawn(async move { claim_story(&store_a, &mine, &input_a).await });
        let c = tokio::spawn(async move { claim_story(&store_b, &theirs, &input_b).await });
        let results = [a.await.unwrap(), c.await.unwrap()];

        let winners: Vec<&StoryClaim> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
        assert_eq!(winners.len(), 1, "ronda {round}: {results:?}");
        let loser = results.iter().find_map(|r| r.as_ref().err()).unwrap();
        assert_eq!(loser.code(), ErrorCode::Claimed, "ronda {round}");
        assert_eq!(b.open_claims(&s).await, 1);
        assert_eq!(
            b.assigned_to(&s).await,
            Some(winners[0].owner.agent_id.clone())
        );
        // Libera para que el tope de reclamos por agente no intervenga en la carrera.
        b.release(winners[0]).await.unwrap();
    }
}

#[tokio::test]
async fn only_ready_stories_of_the_session_project_can_be_claimed() {
    use ColumnCategory::{Active, Backlog};
    let b = Board::new().await;

    let in_progress = b.story("En curso", Active).await;
    assert_eq!(
        code(claim_story(&b.store, &b.mine, &by_id(&in_progress)).await),
        ErrorCode::NotReady
    );
    let blocked = b.ready("Bloqueada").await;
    set_story_blocker(
        &b.store,
        &Actor::Human,
        &SetBlockerInput {
            project_id: b.project.id.clone(),
            story_id: blocked.id.clone(),
            reason: Some("Falta acceso".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        code(claim_story(&b.store, &b.mine, &by_id(&blocked)).await),
        ErrorCode::NotReady
    );
    let prerequisite = b.story("Base", Backlog).await;
    let waiting = b.ready("Espera").await;
    add_story_dependency(
        &b.store,
        &DependencyInput {
            project_id: b.project.id.clone(),
            story_id: waiting.id.clone(),
            depends_on_id: prerequisite.id.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        code(claim_story(&b.store, &b.mine, &by_id(&waiting)).await),
        ErrorCode::NotReady
    );
    let assigned = b.ready("De otro").await;
    b.assign(&assigned, Some(&b.other)).await;
    assert_eq!(
        code(claim_story(&b.store, &b.mine, &by_id(&assigned)).await),
        ErrorCode::NotReady
    );
    // El backlog cuenta como listo (decisión de HU-013).
    claim_story(&b.store, &b.mine, &by_id(&prerequisite))
        .await
        .unwrap();

    // Historia de otro proyecto: «no encontrada», por id y por número.
    let foreign_project = project(&b.store, "Ajeno").await;
    let foreign = create_story(
        &b.store,
        UI,
        &CreateStoryInput {
            request_id: rid(),
            project_id: foreign_project.id.clone(),
            title: "Ajena".into(),
            column_id: None,
            priority: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        code(claim_story(&b.store, &b.mine, &by_id(&foreign)).await),
        ErrorCode::NotFound
    );
    let missing = ClaimStoryInput {
        request_id: rid(),
        story_id: None,
        number: Some(999),
    };
    assert_eq!(
        code(claim_story(&b.store, &b.mine, &missing).await),
        ErrorCode::NotFound
    );
    // Ni ninguno ni ambos.
    for (story_id, number) in [(None, None), (Some(in_progress.id.clone()), Some(1))] {
        let input = ClaimStoryInput {
            request_id: rid(),
            story_id,
            number,
        };
        assert_eq!(
            code(claim_story(&b.store, &b.mine, &input).await),
            ErrorCode::Validation
        );
    }
    // Ningún fallo dejó reclamos ni asignaciones a medias.
    for s in [&in_progress, &blocked, &waiting] {
        assert_eq!(b.open_claims(s).await, 0);
        assert_eq!(b.assigned_to(s).await, None);
    }
}

// ---------- CA-2: renovar, vencer y liberar ----------

#[tokio::test]
async fn renewal_and_expiry_follow_the_clock() {
    let b = Board::new().await;
    let s = b.ready("Pagos").await;
    let claim = claim_story(&b.store, &b.mine, &by_id(&s)).await.unwrap();

    b.clock.advance(29 * MINUTE);
    let renewed = renew_claim(&b.store, &b.mine, &renewal(&claim))
        .await
        .unwrap();
    assert_eq!(renewed.claim_id, claim.claim_id);
    let renewed_at = renewed.renewed_at.clone().unwrap();
    assert_eq!(ms_between(&claim.claimed_at, &renewed_at), 29 * 60_000);
    assert_eq!(
        ms_between(&renewed_at, &renewed.expires_at),
        CLAIM_DURATION_MINUTES * 60_000
    );

    // Sin la renovación habría vencido ya; con ella sigue vigente.
    b.clock.advance(29 * MINUTE);
    assert_eq!(
        code(claim_story(&b.store, &b.theirs, &by_id(&s)).await),
        ErrorCode::Claimed
    );

    // Vence: no se puede renovar, deja de verse en la app y el otro agente lo ve libre de reclamo.
    b.clock.advance(2 * MINUTE);
    assert_eq!(
        code(renew_claim(&b.store, &b.mine, &renewal(&claim)).await),
        ErrorCode::ClaimEnded
    );
    assert!(
        list_active_claims(&b.store, &b.project.id)
            .await
            .unwrap()
            .is_empty()
    );
    let context = get_story_context(&b.store, &b.mine, &StoryKey::Id(s.id.clone()))
        .await
        .unwrap();
    assert!(context.your_claim.is_none());

    // La asignación se queda (decisión del propietario): el mismo agente puede reclamarla de
    // nuevo, con un reclamo nuevo; el vencido queda cerrado como «expired».
    let second = claim_story(&b.store, &b.mine, &by_id(&s)).await.unwrap();
    assert_ne!(second.claim_id, claim.claim_id);
    let reason: String =
        sqlx::query_scalar("SELECT end_reason FROM story_claims WHERE id = ? AND ended_at_ms > 0")
            .bind(&claim.claim_id)
            .fetch_one(b.store.reader())
            .await
            .unwrap();
    assert_eq!(reason, "expired");
    assert_eq!(b.open_claims(&s).await, 1);
}

#[tokio::test]
async fn expiry_keeps_the_assignment_so_only_the_same_agent_reclaims() {
    let b = Board::new().await;
    let s = b.ready("Informe").await;
    claim_story(&b.store, &b.mine, &by_id(&s)).await.unwrap();
    let context = get_story_context(&b.store, &b.theirs, &StoryKey::Id(s.id.clone()))
        .await
        .unwrap();
    assert!(context.claimed_by_other);
    assert_eq!(context.assignment, "other");

    // Vence: el reclamo ya no cuenta, pero la asignación (decisión del propietario) sí.
    b.clock.advance(31 * MINUTE);
    let context = get_story_context(&b.store, &b.theirs, &StoryKey::Id(s.id.clone()))
        .await
        .unwrap();
    assert!(!context.claimed_by_other);
    assert!(!context.ready);
    assert_eq!(
        code(claim_story(&b.store, &b.theirs, &by_id(&s)).await),
        ErrorCode::NotReady
    );
    // La persona quita la asignación: ahora otro agente la ve lista y la reclama.
    b.assign(&s, None).await;
    assert!(b.ready_ids(&b.theirs).await.contains(&s.id));
    let theirs = claim_story(&b.store, &b.theirs, &by_id(&s)).await.unwrap();
    assert_eq!(theirs.owner.agent_id, b.other.id);
    assert_eq!(b.assigned_to(&s).await, Some(b.other.id.clone()));
}

#[tokio::test]
async fn changing_or_removing_the_assignment_ends_the_previous_claim() {
    let b = Board::new().await;
    // Reasignar a otro agente: mi reclamo termina y el nuevo puede reclamarla ya.
    let reassigned = b.ready("Reasignada").await;
    let mine = claim_story(&b.store, &b.mine, &by_id(&reassigned))
        .await
        .unwrap();
    b.assign(&reassigned, Some(&b.other)).await;
    assert_eq!(
        code(renew_claim(&b.store, &b.mine, &renewal(&mine)).await),
        ErrorCode::ClaimEnded
    );
    claim_story(&b.store, &b.theirs, &by_id(&reassigned))
        .await
        .unwrap();

    // Quitar la asignación: también termina; cualquiera puede reclamarla.
    let unassigned = b.ready("Sin asignar").await;
    let mine = claim_story(&b.store, &b.mine, &by_id(&unassigned))
        .await
        .unwrap();
    b.assign(&unassigned, None).await;
    assert_eq!(b.open_claims(&unassigned).await, 0);
    assert_eq!(
        code(renew_claim(&b.store, &b.mine, &renewal(&mine)).await),
        ErrorCode::ClaimEnded
    );
    claim_story(&b.store, &b.theirs, &by_id(&unassigned))
        .await
        .unwrap();

    // Reasignar al mismo agente que la tiene reclamada no cambia nada.
    let same = b.ready("Misma").await;
    let mine = claim_story(&b.store, &b.mine, &by_id(&same)).await.unwrap();
    b.assign(&same, Some(&b.me)).await;
    renew_claim(&b.store, &b.mine, &renewal(&mine))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_agent_cannot_hoard_more_than_the_limit_of_claims() {
    let b = Board::new().await;
    let mut claims = Vec::new();
    for i in 0..MAX_OPEN_CLAIMS {
        let s = b.ready(&format!("Historia {i}")).await;
        claims.push(claim_story(&b.store, &b.mine, &by_id(&s)).await.unwrap());
    }
    let extra = b.ready("Una más").await;
    let error = claim_story(&b.store, &b.mine, &by_id(&extra))
        .await
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ClaimLimit);
    assert_eq!(b.open_claims(&extra).await, 0);
    assert_eq!(b.assigned_to(&extra).await, None);
    // Volver a pedir una que ya tiene no cuenta como nueva; el otro agente no está limitado.
    let first = by_id(&StorySummary {
        id: claims[0].story_id.clone(),
        ..extra.clone()
    });
    claim_story(&b.store, &b.mine, &first).await.unwrap();
    claim_story(&b.store, &b.theirs, &by_id(&extra))
        .await
        .unwrap();
    // Al liberarse uno, vuelve a haber hueco.
    b.release(&claims[0]).await.unwrap();
    let another = b.ready("Otra más").await;
    claim_story(&b.store, &b.mine, &by_id(&another))
        .await
        .unwrap();
}

#[tokio::test]
async fn archived_or_accepted_stories_cannot_be_renewed() {
    let b = Board::new().await;
    let s = b.ready("Archivable").await;
    let claim = claim_story(&b.store, &b.mine, &by_id(&s)).await.unwrap();
    let current = list_stories(&b.store, &b.project.id, false)
        .await
        .unwrap()
        .into_iter()
        .find(|x| x.id == s.id)
        .unwrap();
    archive_story(
        &b.store,
        &ArchiveStoryInput {
            project_id: b.project.id.clone(),
            id: s.id.clone(),
            expected_revision: current.revision,
        },
    )
    .await
    .unwrap();
    let ended = renew_claim(&b.store, &b.mine, &renewal(&claim))
        .await
        .unwrap_err();
    assert!(matches!(ended, CoreError::ClaimEnded { expired: false }));
    // Ya no aparece en la app ni ocupa hueco del tope.
    assert!(
        list_active_claims(&b.store, &b.project.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// La cláusula de reclamos ajenos de la regla, aislada: un reclamo vigente de otro agente
/// sobre una historia sin asignar (estado que hoy no produce ningún caso de uso, porque
/// reclamar asigna; la regla no debe depender de ello).
#[tokio::test]
async fn a_live_claim_of_another_agent_hides_the_story_even_without_assignment() {
    let b = Board::new().await;
    let s = b.ready("Sin asignar").await;
    let other_credential: String =
        sqlx::query_scalar("SELECT id FROM agent_credentials WHERE agent_id = ?")
            .bind(&b.other.id)
            .fetch_one(b.store.reader())
            .await
            .unwrap();
    let (project_id, story_id, agent_id) = (b.project.id.clone(), s.id.clone(), b.other.id.clone());
    b.store
        .write(move |conn| {
            Box::pin(async move {
                sqlx::query(
                    "INSERT INTO story_claims (id, project_id, story_id, agent_id, \
                     credential_id, claimed_at_ms, expires_at_ms) VALUES ('k', ?, ?, ?, ?, ?, ?)",
                )
                .bind(project_id)
                .bind(story_id)
                .bind(agent_id)
                .bind(other_credential)
                .bind(START_MS)
                .bind(START_MS + 30 * 60_000)
                .execute(&mut *conn)
                .await?;
                Ok(())
            })
        })
        .await
        .unwrap();
    assert_eq!(b.assigned_to(&s).await, None);
    assert!(!b.ready_ids(&b.mine).await.contains(&s.id));
    let context = get_story_context(&b.store, &b.mine, &StoryKey::Id(s.id.clone()))
        .await
        .unwrap();
    assert!(!context.ready);
    assert!(context.claimed_by_other);
    assert_eq!(
        code(claim_story(&b.store, &b.mine, &by_id(&s)).await),
        ErrorCode::Claimed
    );
    // Al vencer deja de contar.
    b.clock.advance(31 * MINUTE);
    assert!(b.ready_ids(&b.mine).await.contains(&s.id));
}

#[tokio::test]
async fn restoring_a_backup_never_revives_its_claims() {
    let b = Board::new().await;
    let s = b.ready("Antes de la copia").await;
    let claim = claim_story(&b.store, &b.mine, &by_id(&s)).await.unwrap();
    let backup = b.store.create_backup(BackupKind::Manual).await.unwrap();
    b.release(&claim).await.unwrap();
    let Board {
        _data: data,
        store,
        mine,
        ..
    } = b;
    store.close().await;
    drop(store);

    restore_backup(data.path(), &backup_path(data.path(), &backup.id).unwrap())
        .await
        .unwrap();
    let store = Store::open(data.path()).await.unwrap();
    // En la copia el reclamo estaba abierto; tras restaurar queda cerrado.
    let open: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM story_claims WHERE ended_at_ms IS NULL")
            .fetch_one(store.reader())
            .await
            .unwrap();
    assert_eq!(open, 0);
    assert_eq!(
        code(renew_claim(&store, &mine, &renewal(&claim)).await),
        ErrorCode::ClaimEnded
    );
    // La historia sigue asignada al agente, que puede volver a reclamarla.
    claim_story(&store, &mine, &by_id(&s)).await.unwrap();
}

#[tokio::test]
async fn only_the_owner_renews_and_only_a_person_releases() {
    let b = Board::new().await;
    let s = b.ready("Buscador").await;
    let claim = claim_story(&b.store, &b.mine, &by_id(&s)).await.unwrap();

    // Otro agente, o el mismo agente con una credencial de otro proyecto: «no encontrado».
    assert_eq!(
        code(renew_claim(&b.store, &b.theirs, &renewal(&claim)).await),
        ErrorCode::NotFound
    );
    let elsewhere = project(&b.store, "Otro proyecto").await;
    let me_elsewhere = session(&b.store, &elsewhere, &b.me).await;
    assert_eq!(
        code(renew_claim(&b.store, &me_elsewhere, &renewal(&claim)).await),
        ErrorCode::NotFound
    );
    // Otra credencial del mismo agente y proyecto sí puede (la identidad es el agente).
    let me_again = session(&b.store, &b.project, &b.me).await;
    renew_claim(&b.store, &me_again, &renewal(&claim))
        .await
        .unwrap();

    // Un agente no libera reclamos; una persona sí, solo desde su proyecto.
    let as_agent = release_claim(
        &b.store,
        &b.mine.actor(),
        &ReleaseClaimInput {
            project_id: b.project.id.clone(),
            claim_id: claim.claim_id.clone(),
        },
    )
    .await;
    assert_eq!(code(as_agent), ErrorCode::Forbidden);
    let wrong_project = release_claim(
        &b.store,
        &Actor::Human,
        &ReleaseClaimInput {
            project_id: elsewhere.id.clone(),
            claim_id: claim.claim_id.clone(),
        },
    )
    .await;
    assert_eq!(code(wrong_project), ErrorCode::NotFound);
    assert_eq!(b.open_claims(&s).await, 1);

    b.release(&claim).await.unwrap();
    b.release(&claim).await.unwrap(); // idempotente
    let ended = renew_claim(&b.store, &b.mine, &renewal(&claim))
        .await
        .unwrap_err();
    assert!(matches!(ended, CoreError::ClaimEnded { expired: false }));
    assert!(
        list_active_claims(&b.store, &b.project.id)
            .await
            .unwrap()
            .is_empty()
    );
    // Liberar quita también la asignación (decisión del propietario): queda libre para todos.
    assert_eq!(b.assigned_to(&s).await, None);
    assert!(b.ready_ids(&b.theirs).await.contains(&s.id));
    claim_story(&b.store, &b.theirs, &by_id(&s)).await.unwrap();
}

// ---------- CA-3: reintentos e identidad ----------

#[tokio::test]
async fn retries_with_the_same_request_id_replay_and_other_payloads_conflict() {
    let b = Board::new().await;
    let first = b.ready("Primera").await;
    let second = b.ready("Segunda").await;
    let input = by_id(&first);

    let claim = claim_story(&b.store, &b.mine, &input).await.unwrap();
    // La respuesta se perdió y el agente reintenta, incluso tras liberar: mismo resultado,
    // sin un segundo reclamo.
    b.release(&claim).await.unwrap();
    let replay = claim_story(&b.store, &b.mine, &input).await.unwrap();
    assert_eq!(replay, claim);
    assert_eq!(b.open_claims(&first).await, 0);

    // Mismo requestId con otra historia o para renovar → conflicto, sin efecto.
    let other_story = ClaimStoryInput {
        request_id: input.request_id.clone(),
        story_id: Some(second.id.clone()),
        number: None,
    };
    assert_eq!(
        code(claim_story(&b.store, &b.mine, &other_story).await),
        ErrorCode::IdempotencyConflict
    );
    assert_eq!(b.open_claims(&second).await, 0);
    let as_renewal = RenewClaimInput {
        request_id: input.request_id.clone(),
        claim_id: claim.claim_id.clone(),
    };
    assert_eq!(
        code(renew_claim(&b.store, &b.mine, &as_renewal).await),
        ErrorCode::IdempotencyConflict
    );

    // El recibo es de la credencial: el mismo requestId de otro agente es otra petición.
    let theirs = claim_story(
        &b.store,
        &b.theirs,
        &ClaimStoryInput {
            request_id: input.request_id.clone(),
            story_id: Some(second.id.clone()),
            number: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(theirs.owner.agent_id, b.other.id);

    // Renovar con el mismo requestId tras avanzar el reloj devuelve la renovación original.
    let renew_input = renewal(&theirs);
    let renewed = renew_claim(&b.store, &b.theirs, &renew_input)
        .await
        .unwrap();
    b.clock.advance(5 * MINUTE);
    assert_eq!(
        renew_claim(&b.store, &b.theirs, &renew_input)
            .await
            .unwrap(),
        renewed
    );
    // requestId mal formado.
    let bad = ClaimStoryInput {
        request_id: "con espacio".into(),
        story_id: Some(second.id.clone()),
        number: None,
    };
    assert_eq!(
        code(claim_story(&b.store, &b.mine, &bad).await),
        ErrorCode::Validation
    );
}

// ---------- Utilidades ----------

async fn project(store: &Store, name: &str) -> Project {
    create_project(
        store,
        UI,
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
        UI,
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

async fn session(store: &Store, p: &Project, a: &Agent) -> AgentSession {
    let created = create_agent_credential(
        store,
        &Actor::Human,
        &CreateAgentCredentialInput {
            project_id: p.id.clone(),
            agent_id: a.id.clone(),
        },
    )
    .await
    .unwrap();
    authenticate_agent(store, &created.token).await.unwrap()
}
