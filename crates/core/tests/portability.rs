//! HU-010: exportar e importar la planificación como carpeta Markdown. Base SQLite temporal real
//! y carpetas temporales reales.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use kanbania_core::error::ErrorCode;
use kanbania_core::features::columns::{
    ColumnCategory, UpdateColumnInput, list_columns, update_column,
};
use kanbania_core::features::epics::{CreateEpicInput, create_epic};
use kanbania_core::features::portability::{
    ApplyImportInput, ExportProjectInput, ImportAction, ImportPreview, ImportResolution,
    ResolutionChoice, apply_import, export_project, preview_import,
};
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::relations::{
    DependencyInput, SetStoryAgentInput, SetStoryEpicInput, SetStoryParentInput,
    add_story_dependency, set_story_agent, set_story_epic, set_story_parent,
};
use kanbania_core::features::repositories::{LinkRepositoryInput, link_repository};
use kanbania_core::features::review::{
    AcceptStoryInput, SetBlockerInput, accept_story, set_story_blocker,
};
use kanbania_core::features::stories::{
    ArchiveStoryInput, CreateStoryInput, CriterionDraft, StoryDetail, UpdateStoryInput,
    archive_story, create_story, get_story, list_stories, update_story,
};
use kanbania_core::features::team::{AgentFields, AvatarSpec, CreateAgentInput, create_agent};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::storage::Store;

const HUMAN: Actor = Actor::Human;

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Descripción con Unicode, Markdown y trampas para el formato (`+++`, CRLF no, fences).
const RICH: &str = "Ñandú 🦤 «comillas» — 漢字\n\n```toml\n+++\nx = 1\n+++\n```\n\n- [ ] tarea\n\n<script>alert(1)</script>\n";

struct Env {
    _dir: tempfile::TempDir,
    store: Store,
}

impl Env {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        Self { _dir: dir, store }
    }

    async fn project(&self, name: &str) -> Project {
        create_project(
            &self.store,
            "ui",
            &CreateProjectInput {
                request_id: rid(),
                name: name.into(),
            },
        )
        .await
        .unwrap()
    }

    async fn story(
        &self,
        project: &str,
        title: &str,
        description: &str,
        criteria: &[&str],
    ) -> StoryDetail {
        let s = create_story(
            &self.store,
            "ui",
            &CreateStoryInput {
                request_id: rid(),
                project_id: project.into(),
                title: title.into(),
                column_id: None,
                priority: None,
            },
        )
        .await
        .unwrap();
        let d = get_story(&self.store, project, &s.id).await.unwrap();
        update_story(
            &self.store,
            &UpdateStoryInput {
                project_id: project.into(),
                id: s.id.clone(),
                expected_revision: d.revision,
                title: title.into(),
                description: description.into(),
                priority: d.priority,
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

    async fn export(&self, project: &str, to: &Path) -> PathBuf {
        let result = export_project(
            &self.store,
            &HUMAN,
            &ExportProjectInput {
                project_id: project.into(),
                destination: to.to_string_lossy().into(),
            },
            "test",
        )
        .await
        .unwrap();
        to.join(result.folder_name)
    }

    async fn preview(
        &self,
        package: &Path,
    ) -> Result<ImportPreview, kanbania_core::error::CoreError> {
        preview_import(&self.store, &HUMAN, &package.to_string_lossy()).await
    }

    async fn apply(
        &self,
        package: &Path,
        resolutions: Vec<ImportResolution>,
    ) -> Result<kanbania_core::features::portability::ImportResult, kanbania_core::error::CoreError>
    {
        let preview = self.preview(package).await?;
        apply_import(
            &self.store,
            &HUMAN,
            &ApplyImportInput {
                path: package.to_string_lossy().into(),
                fingerprint: preview.fingerprint,
                resolutions,
            },
        )
        .await
    }

    /// Huella de todo el contenido de la base relevante, para comprobar «sin cambios».
    async fn fingerprint(&self) -> String {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT group_concat(x, '|') FROM ( \
               SELECT id || title || revision || ifnull(position, '-') AS x FROM stories \
               UNION ALL SELECT id || title || revision FROM epics \
               UNION ALL SELECT id || name FROM projects \
               UNION ALL SELECT id || text FROM acceptance_criteria \
               UNION ALL SELECT story_id || depends_on_id FROM story_dependencies \
               UNION ALL SELECT story_id || parent_id FROM story_parents \
               UNION ALL SELECT story_id || epic_id FROM story_epics \
               UNION ALL SELECT story_id || agent_id FROM story_agents \
               UNION ALL SELECT id || number || position FROM acceptance_criteria \
               UNION ALL SELECT id || name || ifnull(wip_limit, '-') || position FROM columns \
               UNION ALL SELECT id || number || column_id || next_criterion_number FROM stories \
               ORDER BY 1)",
        )
        .fetch_all(self.store.reader())
        .await
        .unwrap();
        format!("{rows:?}")
    }
}

/// Proyecto de ejemplo con todo lo exportable. Devuelve (proyecto, [a, b, c, d]).
async fn sample(env: &Env, repo_parent: &Path) -> (Project, Vec<StoryDetail>) {
    let project = env.project("Planificación ñ").await;
    let p = project.id.as_str();
    let repo = repo_parent.join("repo-secreto");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    link_repository(
        &env.store,
        "ui",
        &LinkRepositoryInput {
            request_id: rid(),
            project_id: p.into(),
            path: repo.to_string_lossy().into(),
        },
    )
    .await
    .unwrap();
    let epic = create_epic(
        &env.store,
        "ui",
        &CreateEpicInput {
            request_id: rid(),
            project_id: p.into(),
            title: "Acceso 🔐".into(),
            description: Some("Épica *Markdown*".into()),
        },
    )
    .await
    .unwrap();
    let a = env
        .story(
            p,
            "Inicio de sesión ñ",
            RICH,
            &["Se ve «bien»", "Funciona 🚀"],
        )
        .await;
    let b = env.story(p, "API", "", &["Responde"]).await;
    let c = env.story(p, "Subtarea", "texto", &["Hecha"]).await;
    let d = env.story(p, "Antigua", "archivar", &["x"]).await;
    set_story_epic(
        &env.store,
        &SetStoryEpicInput {
            project_id: p.into(),
            story_id: a.id.clone(),
            epic_id: Some(epic.id.clone()),
        },
    )
    .await
    .unwrap();
    set_story_parent(
        &env.store,
        &SetStoryParentInput {
            project_id: p.into(),
            story_id: c.id.clone(),
            parent_id: Some(a.id.clone()),
        },
    )
    .await
    .unwrap();
    add_story_dependency(
        &env.store,
        &DependencyInput {
            project_id: p.into(),
            story_id: a.id.clone(),
            depends_on_id: b.id.clone(),
        },
    )
    .await
    .unwrap();
    let agent = create_agent(
        &env.store,
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
    set_story_agent(
        &env.store,
        &SetStoryAgentInput {
            project_id: p.into(),
            story_id: a.id.clone(),
            agent_id: Some(agent.id.clone()),
        },
    )
    .await
    .unwrap();
    set_story_blocker(
        &env.store,
        &HUMAN,
        &SetBlockerInput {
            project_id: p.into(),
            story_id: a.id.clone(),
            reason: Some("Falta acceso".into()),
        },
    )
    .await
    .unwrap();
    let b_now = get_story(&env.store, p, &b.id).await.unwrap();
    accept_story(
        &env.store,
        &HUMAN,
        &AcceptStoryInput {
            project_id: p.into(),
            story_id: b.id.clone(),
            expected_revision: b_now.revision,
            satisfied_criteria: b_now.criteria.iter().map(|c| c.id.clone()).collect(),
            column_id: None,
            to_index: None,
        },
    )
    .await
    .unwrap();
    archive_story(
        &env.store,
        &ArchiveStoryInput {
            project_id: p.into(),
            id: d.id.clone(),
            expected_revision: d.revision,
        },
    )
    .await
    .unwrap();
    (project, vec![a, b, c, d])
}

fn read_all(dir: &Path) -> String {
    let mut out = String::new();
    for sub in ["", "stories", "epics"] {
        for entry in std::fs::read_dir(dir.join(sub)).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                out.push_str(&std::fs::read_to_string(path).unwrap());
            }
        }
    }
    out
}

fn story_file(package: &Path, number: i64) -> PathBuf {
    let prefix = format!("{number:04}-");
    std::fs::read_dir(package.join("stories"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&prefix)
        })
        .unwrap()
}

fn edit(path: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains(from), "{from} no está en {}", path.display());
    std::fs::write(path, text.replacen(from, to, 1)).unwrap();
}

#[tokio::test]
async fn round_trip_to_a_new_install_keeps_unicode_markdown_ids_and_relations() {
    let source = Env::new().await;
    let work = tempfile::tempdir().unwrap();
    let (project, stories) = sample(&source, work.path()).await;
    let package = source.export(&project.id, work.path()).await;

    // Sin rutas absolutas ni repositorios; con manifiesto versionado.
    let all = read_all(&package);
    assert!(!all.contains("repo-secreto"), "no se exportan repositorios");
    assert!(!all.contains(&work.path().to_string_lossy().to_string()));
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(package.join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["format"], "kanbania-package");
    assert_eq!(manifest["version"], 1);
    assert_eq!(manifest["stories"].as_array().unwrap().len(), 4);

    let target = Env::new().await;
    let preview = target.preview(&package).await.unwrap();
    assert!(!preview.project_exists);
    assert!(
        preview
            .items
            .iter()
            .all(|i| i.action == ImportAction::Create)
    );
    assert!(
        preview.warnings.iter().any(|w| w.contains("Robota")),
        "{:?}",
        preview.warnings
    );
    assert!(
        preview
            .warnings
            .iter()
            .any(|w| w.contains("aceptación no se importa"))
    );
    let result = target.apply(&package, vec![]).await.unwrap();
    assert_eq!(result.created, 5);

    let [a, b, c, d] = stories.as_slice() else {
        panic!()
    };
    let a2 = get_story(&target.store, &project.id, &a.id).await.unwrap();
    assert_eq!(a2.description, a.description, "descripción exacta");
    assert_eq!(a.description, RICH.trim_end());
    assert_eq!(a2.title, a.title);
    assert_eq!(a2.number, a.number);
    let texts: Vec<_> = a2
        .criteria
        .iter()
        .map(|c| (c.number, c.text.clone()))
        .collect();
    let before: Vec<_> = a
        .criteria
        .iter()
        .map(|c| (c.number, c.text.clone()))
        .collect();
    assert_eq!(texts, before, "criterios CA-n estables");

    let list = list_stories(&target.store, &project.id, true)
        .await
        .unwrap();
    let find = |id: &str| list.iter().find(|s| s.id == id).unwrap();
    assert!(find(&a.id).epic_id.is_some());
    assert_eq!(find(&c.id).parent_id.as_deref(), Some(a.id.as_str()));
    assert_eq!(
        find(&a.id).pending_prerequisites,
        1,
        "b no está aceptada aquí"
    );
    assert!(
        find(&a.id).agent_id.is_none(),
        "el agente no existe en el destino"
    );
    assert!(
        find(&a.id).blocker_reason.is_none(),
        "los bloqueos no se importan"
    );
    // La aceptada no se importa como aceptada ni entra en «hecho».
    let review = list_columns(&target.store, &project.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.category == ColumnCategory::Review)
        .unwrap();
    assert!(find(&b.id).accepted_at.is_none());
    assert_eq!(find(&b.id).column_id, review.id);
    assert!(find(&d.id).archived_at.is_some());
}

#[tokio::test]
async fn reimport_does_not_duplicate_and_three_way_merge_detects_each_side() {
    let env = Env::new().await;
    let work = tempfile::tempdir().unwrap();
    let (project, stories) = sample(&env, work.path()).await;
    let package = env.export(&project.id, work.path()).await;
    let a = &stories[0];
    let c = &stories[2];

    // Reimportar sin cambios: todo igual, nada se duplica ni cambia.
    let before = env.fingerprint().await;
    let preview = env.preview(&package).await.unwrap();
    assert!(preview.project_exists);
    assert!(
        preview
            .items
            .iter()
            .all(|i| i.action == ImportAction::Unchanged)
    );
    let result = env.apply(&package, vec![]).await.unwrap();
    assert_eq!((result.created, result.updated), (0, 0));
    assert_eq!(env.fingerprint().await, before);

    // Solo cambia el paquete → actualizar (incluido un criterio nuevo escrito a mano).
    edit(
        &story_file(&package, a.number),
        "title = \"Inicio de sesión ñ\"",
        "title = \"Acceso con clave\"",
    );
    edit(
        &story_file(&package, a.number),
        "[[criteria]]",
        "[[criteria]]\ntext = \"Nuevo a mano\"\n\n[[criteria]]",
    );
    let preview = env.preview(&package).await.unwrap();
    let item = preview.items.iter().find(|i| i.id == a.id).unwrap();
    assert_eq!(item.action, ImportAction::Update);
    env.apply(&package, vec![]).await.unwrap();
    let a_now = get_story(&env.store, &project.id, &a.id).await.unwrap();
    assert_eq!(a_now.title, "Acceso con clave");
    assert_eq!(a_now.criteria[0].text, "Nuevo a mano");
    assert_eq!(a_now.criteria[0].number, 3, "número nuevo, sin reutilizar");
    // Reaplicar el mismo paquete ya no cambia nada.
    let again = env.preview(&package).await.unwrap();
    assert_eq!(
        again.items.iter().find(|i| i.id == a.id).unwrap().action,
        ImportAction::Unchanged
    );

    // Solo cambia lo local → se conserva lo local.
    let second = work.path().join("second");
    std::fs::create_dir(&second).unwrap();
    let fresh = env.export(&project.id, &second).await;
    let c_now = get_story(&env.store, &project.id, &c.id).await.unwrap();
    update_story(
        &env.store,
        &UpdateStoryInput {
            project_id: project.id.clone(),
            id: c.id.clone(),
            expected_revision: c_now.revision,
            title: "Subtarea local".into(),
            description: c_now.description.clone(),
            priority: c_now.priority,
            criteria: c_now
                .criteria
                .iter()
                .map(|x| CriterionDraft {
                    id: Some(x.id.clone()),
                    text: x.text.clone(),
                })
                .collect(),
        },
    )
    .await
    .unwrap();
    let preview = env.preview(&fresh).await.unwrap();
    assert_eq!(
        preview.items.iter().find(|i| i.id == c.id).unwrap().action,
        ImportAction::KeepLocal
    );

    // Cambian ambos → conflicto explícito.
    edit(
        &story_file(&fresh, c.number),
        "title = \"Subtarea\"",
        "title = \"Subtarea paquete\"",
    );
    let preview = env.preview(&fresh).await.unwrap();
    let conflict = preview.items.iter().find(|i| i.id == c.id).unwrap();
    assert_eq!(conflict.action, ImportAction::Conflict);
    assert_eq!(
        conflict.local_label.as_deref(),
        Some(format!("#{} Subtarea local", c.number).as_str())
    );

    // Sin resolver: no se aplica nada.
    let before = env.fingerprint().await;
    let err = env.apply(&fresh, vec![]).await.unwrap_err();
    assert_eq!(err.code(), ErrorCode::Validation);
    assert_eq!(env.fingerprint().await, before);

    // Elegir lo local lo conserva; elegir el paquete lo sustituye.
    env.apply(
        &fresh,
        vec![ImportResolution {
            id: c.id.clone(),
            choice: ResolutionChoice::Local,
        }],
    )
    .await
    .unwrap();
    assert_eq!(
        get_story(&env.store, &project.id, &c.id)
            .await
            .unwrap()
            .title,
        "Subtarea local"
    );
    env.apply(
        &fresh,
        vec![ImportResolution {
            id: c.id.clone(),
            choice: ResolutionChoice::Package,
        }],
    )
    .await
    .unwrap();
    assert_eq!(
        get_story(&env.store, &project.id, &c.id)
            .await
            .unwrap()
            .title,
        "Subtarea paquete"
    );
}

#[tokio::test]
async fn invalid_packages_change_nothing() {
    let env = Env::new().await;
    let work = tempfile::tempdir().unwrap();
    let (project, stories) = sample(&env, work.path()).await;
    let package = env.export(&project.id, work.path()).await;
    let before = env.fingerprint().await;
    let a = &stories[0];
    let b = &stories[1];

    // Cabecera TOML rota.
    let broken = work.path().join("broken");
    copy_dir(&package, &broken);
    edit(&story_file(&broken, a.number), "title = ", "title = = ");
    let err = env.preview(&broken).await.unwrap_err();
    assert_eq!(err.code(), ErrorCode::Validation);

    // Ciclo de dependencias en el paquete: la transacción entera se deshace.
    let cyclic = work.path().join("cyclic");
    copy_dir(&package, &cyclic);
    edit(
        &story_file(&cyclic, b.number),
        "nextCriterionNumber",
        &format!("dependsOn = [\"{}\"]\nnextCriterionNumber", a.id),
    );
    edit(&story_file(&cyclic, b.number), "dependsOn = []\n", "");
    edit(
        &story_file(&cyclic, b.number),
        "title = \"API\"",
        "title = \"API cambiada\"",
    );
    let err = env.apply(&cyclic, vec![]).await.unwrap_err();
    assert_eq!(err.code(), ErrorCode::Cycle, "{err:?}");

    // Formato y versión incompatibles.
    let future = work.path().join("future");
    copy_dir(&package, &future);
    edit(
        &future.join("manifest.json"),
        "\"version\": 1",
        "\"version\": 2",
    );
    let err = env.preview(&future).await.unwrap_err();
    assert!(err.to_string().contains("versión 2"), "{err}");
    edit(
        &future.join("manifest.json"),
        "kanbania-package",
        "otra-cosa",
    );
    assert!(
        env.preview(&future)
            .await
            .unwrap_err()
            .to_string()
            .contains("incompatible")
    );

    // Campo desconocido (esquema estricto).
    let extra = work.path().join("extra");
    copy_dir(&package, &extra);
    edit(
        &extra.join("manifest.json"),
        "\"format\"",
        "\"hack\": true,\n  \"format\"",
    );
    assert_eq!(
        env.preview(&extra).await.unwrap_err().code(),
        ErrorCode::Validation
    );

    // El paquete cambió entre vista previa y aplicación.
    let preview = env.preview(&package).await.unwrap();
    edit(
        &story_file(&package, a.number),
        "title = ",
        "title = \"X\"\n#",
    );
    let err = apply_import(
        &env.store,
        &HUMAN,
        &ApplyImportInput {
            path: package.to_string_lossy().into(),
            fingerprint: preview.fingerprint,
            resolutions: vec![],
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Validation);

    assert_eq!(
        env.fingerprint().await,
        before,
        "ningún paquete inválido cambió la base"
    );
}

#[tokio::test]
async fn path_traversal_symlinks_and_id_collisions_are_rejected() {
    let env = Env::new().await;
    let work = tempfile::tempdir().unwrap();
    let (project, stories) = sample(&env, work.path()).await;
    let package = env.export(&project.id, work.path()).await;
    let file = story_file(&package, stories[0].number);
    let name = format!("stories/{}", file.file_name().unwrap().to_string_lossy());
    std::fs::write(work.path().join("secret.md"), "+++\nid = \"x\"\n+++\n").unwrap();

    for (index, bad) in [
        "stories/../../secret.md".to_owned(),
        "../secret.md".to_owned(),
        "/etc/passwd".to_owned(),
        "stories\\\\..\\\\secret.md".to_owned(),
        "stories/sub/x.md".to_owned(),
        "stories/.oculto.md".to_owned(),
        "epics/../stories/x.md".to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        let dir = work.path().join(format!("t{index}"));
        copy_dir(&package, &dir);
        edit(&dir.join("manifest.json"), &name, &bad);
        let err = env.preview(&dir).await.unwrap_err();
        assert!(
            err.to_string().contains("Ruta no permitida"),
            "{bad}: {err}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    // Enlace simbólico hacia fuera del paquete.
    #[cfg(unix)]
    {
        let dir = work.path().join("symlink");
        copy_dir(&package, &dir);
        let target = dir.join(&name);
        std::fs::remove_file(&target).unwrap();
        std::os::unix::fs::symlink(work.path().join("secret.md"), &target).unwrap();
        let err = env.preview(&dir).await.unwrap_err();
        assert!(err.to_string().contains("enlace"), "{err}");
    }

    // Ids duplicados dentro del paquete.
    let dir = work.path().join("dup");
    copy_dir(&package, &dir);
    edit(&dir.join("manifest.json"), &stories[1].id, &stories[0].id);
    assert!(
        env.preview(&dir)
            .await
            .unwrap_err()
            .to_string()
            .contains("duplicado")
    );

    // Ids de otro proyecto: el paquete con otro id de proyecto no puede apropiarse de historias.
    let dir = work.path().join("other-project");
    copy_dir(&package, &dir);
    edit(
        &dir.join("manifest.json"),
        &format!("\"id\": \"{}\"", project.id),
        "\"id\": \"otro-proyecto\"",
    );
    let err = env.preview(&dir).await.unwrap_err();
    assert!(err.to_string().contains("otro proyecto"), "{err}");
}

#[tokio::test]
async fn new_stories_in_an_existing_project_get_free_numbers_and_respect_wip() {
    let env = Env::new().await;
    let work = tempfile::tempdir().unwrap();
    let project = env.project("Destino").await;
    let one = env.story(&project.id, "Una", "", &["a"]).await;
    let package = env.export(&project.id, work.path()).await;

    // Una historia nueva escrita a mano con un número ya usado: se renumera.
    let new_id = "0199aaaa-0000-7000-8000-000000000001";
    let old_file = story_file(&package, one.number);
    let text = std::fs::read_to_string(&old_file).unwrap();
    let text = text
        .replace(&one.id, new_id)
        .replace("title = \"Una\"", "title = \"Nueva\"");
    let text = text.split("[[criteria]]").next().unwrap().to_owned() + "+++\nDescripción\n";
    std::fs::write(package.join("stories/0001-nueva.md"), text).unwrap();
    // Sustituye a «Una» en el paquete: el paquete solo trae la nueva, con el #1 ya usado aquí.
    let old_name = format!(
        "stories/{}",
        old_file.file_name().unwrap().to_string_lossy()
    );
    std::fs::remove_file(old_file).unwrap();
    edit(&package.join("manifest.json"), &one.id, new_id);
    edit(
        &package.join("manifest.json"),
        &old_name,
        "stories/0001-nueva.md",
    );
    let preview = env.preview(&package).await.unwrap();
    assert!(
        preview
            .warnings
            .iter()
            .any(|w| w.contains("#1 «Nueva» se importará como #2")),
        "{:?}",
        preview.warnings
    );
    // La columna destino está llena (WIP 1): la importación entera falla sin cambios.
    let backlog = list_columns(&env.store, &project.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.category == ColumnCategory::Backlog)
        .unwrap();
    let limited = update_column(
        &env.store,
        &UpdateColumnInput {
            project_id: project.id.clone(),
            id: backlog.id.clone(),
            expected_revision: backlog.revision,
            name: backlog.name.clone(),
            category: backlog.category,
            wip_limit: Some(1),
        },
    )
    .await
    .unwrap();
    let before = env.fingerprint().await;
    let err = env.apply(&package, vec![]).await.unwrap_err();
    assert_eq!(err.code(), ErrorCode::WipLimitExceeded);
    assert_eq!(env.fingerprint().await, before);
    update_column(
        &env.store,
        &UpdateColumnInput {
            project_id: project.id.clone(),
            id: backlog.id.clone(),
            expected_revision: limited.revision,
            name: backlog.name.clone(),
            category: backlog.category,
            wip_limit: None,
        },
    )
    .await
    .unwrap();

    env.apply(&package, vec![]).await.unwrap();
    let created = get_story(&env.store, &project.id, new_id).await.unwrap();
    assert_eq!(created.number, 2);
    assert_eq!(created.description, "Descripción");
    assert!(created.criteria.is_empty());
}

#[tokio::test]
async fn only_people_can_export_or_import() {
    let env = Env::new().await;
    let work = tempfile::tempdir().unwrap();
    let project = env.project("P").await;
    let agent = Actor::Agent {
        agent_id: "a".into(),
    };
    let err = export_project(
        &env.store,
        &agent,
        &ExportProjectInput {
            project_id: project.id.clone(),
            destination: work.path().to_string_lossy().into(),
        },
        "test",
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Forbidden);
    let package = env.export(&project.id, work.path()).await;
    let err = preview_import(&env.store, &agent, &package.to_string_lossy())
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Forbidden);
    // Exportar dos veces en el mismo segundo no pisa la carpeta existente.
    let again = export_project(
        &env.store,
        &HUMAN,
        &ExportProjectInput {
            project_id: project.id.clone(),
            destination: work.path().to_string_lossy().into(),
        },
        "test",
    )
    .await;
    if let Err(e) = again {
        assert!(e.to_string().contains("Ya existe"), "{e}");
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

async fn set_wip(env: &Env, project: &str, category: ColumnCategory, limit: Option<i64>) {
    let column = list_columns(&env.store, project)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.category == category)
        .unwrap();
    update_column(
        &env.store,
        &UpdateColumnInput {
            project_id: project.into(),
            id: column.id.clone(),
            expected_revision: column.revision,
            name: column.name.clone(),
            category,
            wip_limit: limit,
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn restoring_an_over_limit_board_raises_the_wip_limit_with_a_warning() {
    let source = Env::new().await;
    let work = tempfile::tempdir().unwrap();
    let (project, _) = sample(&source, work.path()).await;
    // Backlog con 2 tarjetas activas (a, c) y límite 1: el tablero ya está por encima.
    set_wip(&source, &project.id, ColumnCategory::Backlog, Some(1)).await;
    // Revisión con límite 1: la aceptada (b) entrará ahí además de lo que haya.
    set_wip(&source, &project.id, ColumnCategory::Review, Some(1)).await;
    let package = source.export(&project.id, work.path()).await;

    let target = Env::new().await;
    let preview = target.preview(&package).await.unwrap();
    assert!(
        preview.warnings.iter().any(|w| w.contains("se sube a 2")),
        "{:?}",
        preview.warnings
    );
    target.apply(&package, vec![]).await.unwrap();
    let backlog = list_columns(&target.store, &project.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.category == ColumnCategory::Backlog)
        .unwrap();
    assert_eq!(backlog.wip_limit, Some(2));
    assert_eq!(backlog.card_count, 2);
}

#[tokio::test]
async fn existing_project_reports_wip_in_the_preview() {
    let env = Env::new().await;
    let work = tempfile::tempdir().unwrap();
    let project = env.project("P").await;
    let one = env.story(&project.id, "Una", "", &["a"]).await;
    let package = env.export(&project.id, work.path()).await;
    // Una historia nueva en el paquete (copia de «Una» con otro id y número).
    let text = std::fs::read_to_string(story_file(&package, one.number)).unwrap();
    let new_id = "0199aaaa-0000-7000-8000-0000000000aa";
    std::fs::write(
        package.join("stories/0002-otra.md"),
        text.replace(&one.id, new_id)
            .replace("number = 1", "number = 2")
            .replace("id = \"", "id = \"x")
            .replacen("id = \"x", "id = \"", 1),
    )
    .unwrap();
    edit(
        &package.join("manifest.json"),
        "\"stories\": [",
        &format!(
            "\"stories\": [\n    {{\"id\": \"{new_id}\", \"file\": \"stories/0002-otra.md\", \
             \"revision\": 1, \"contentHash\": \"\"}},"
        ),
    );
    set_wip(&env, &project.id, ColumnCategory::Backlog, Some(1)).await;
    let err = env.preview(&package).await.unwrap_err();
    assert_eq!(err.code(), ErrorCode::WipLimitExceeded, "{err:?}");
}

#[tokio::test]
async fn out_of_range_numbers_are_rejected() {
    let env = Env::new().await;
    let work = tempfile::tempdir().unwrap();
    let project = env.project("P").await;
    let one = env.story(&project.id, "Una", "", &["a"]).await;
    let package = env.export(&project.id, work.path()).await;
    for (from, to) in [
        ("number = 1\n", "number = 9223372036854775807\n"),
        (
            "nextCriterionNumber = 2",
            "nextCriterionNumber = 9223372036854775807",
        ),
    ] {
        let dir = work.path().join(format!("n{}", to.len()));
        copy_dir(&package, &dir);
        edit(&story_file(&dir, one.number), from, to);
        let err = env.preview(&dir).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation, "{to}");
    }
}

#[tokio::test]
async fn archived_stories_here_are_not_modified_nor_newly_related() {
    let env = Env::new().await;
    let work = tempfile::tempdir().unwrap();
    let project = env.project("P").await;
    let a = env.story(&project.id, "A", "", &["x"]).await;
    let b = env.story(&project.id, "B", "", &["x"]).await;
    let package = env.export(&project.id, work.path()).await;
    let b_now = get_story(&env.store, &project.id, &b.id).await.unwrap();
    archive_story(
        &env.store,
        &ArchiveStoryInput {
            project_id: project.id.clone(),
            id: b.id.clone(),
            expected_revision: b_now.revision,
        },
    )
    .await
    .unwrap();

    // Cambiar la archivada en el paquete: se conserva lo local, con aviso.
    edit(
        &story_file(&package, b.number),
        "title = \"B\"",
        "title = \"B paquete\"",
    );
    let preview = env.preview(&package).await.unwrap();
    assert_eq!(
        preview.items.iter().find(|i| i.id == b.id).unwrap().action,
        ImportAction::KeepLocal
    );
    assert!(
        preview
            .warnings
            .iter()
            .any(|w| w.contains("archivada aquí"))
    );

    // Una dependencia nueva hacia la archivada: rechazada.
    edit(
        &story_file(&package, a.number),
        "dependsOn = []",
        &format!("dependsOn = [\"{}\"]", b.id),
    );
    let err = env.preview(&package).await.unwrap_err();
    assert!(err.to_string().contains("archivada"), "{err}");
}

#[tokio::test]
async fn a_local_change_between_preview_and_apply_requires_a_new_preview() {
    let env = Env::new().await;
    let work = tempfile::tempdir().unwrap();
    let project = env.project("P").await;
    let a = env.story(&project.id, "A", "", &["x"]).await;
    let package = env.export(&project.id, work.path()).await;
    edit(
        &story_file(&package, a.number),
        "title = \"A\"",
        "title = \"A paquete\"",
    );
    let preview = env.preview(&package).await.unwrap();
    assert_eq!(preview.items[0].action, ImportAction::Update);

    // Alguien edita la historia aquí antes de aplicar: ya sería un conflicto no visto.
    let now = get_story(&env.store, &project.id, &a.id).await.unwrap();
    update_story(
        &env.store,
        &UpdateStoryInput {
            project_id: project.id.clone(),
            id: a.id.clone(),
            expected_revision: now.revision,
            title: "A local".into(),
            description: String::new(),
            priority: now.priority,
            criteria: now
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
    let before = env.fingerprint().await;
    let err = apply_import(
        &env.store,
        &HUMAN,
        &ApplyImportInput {
            path: package.to_string_lossy().into(),
            fingerprint: preview.fingerprint,
            resolutions: vec![],
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Validation);
    assert_eq!(env.fingerprint().await, before);
}

#[tokio::test]
async fn crlf_descriptions_round_trip_unchanged_and_bom_is_accepted() {
    let env = Env::new().await;
    let work = tempfile::tempdir().unwrap();
    let project = env.project("P").await;
    let a = env
        .story(&project.id, "A", "línea 1\r\nlínea 2\r\n\r\nfin", &["x"])
        .await;
    let package = env.export(&project.id, work.path()).await;
    let file = story_file(&package, a.number);
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, format!("\u{feff}{text}")).unwrap();
    let preview = env.preview(&package).await.unwrap();
    assert_eq!(
        preview.items[0].action,
        ImportAction::Unchanged,
        "{:?}",
        preview.items
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_stories_folder_is_rejected() {
    let env = Env::new().await;
    let work = tempfile::tempdir().unwrap();
    let project = env.project("P").await;
    env.story(&project.id, "A", "", &["x"]).await;
    let package = env.export(&project.id, work.path()).await;
    let outside = work.path().join("fuera");
    std::fs::rename(package.join("stories"), &outside).unwrap();
    std::os::unix::fs::symlink(&outside, package.join("stories")).unwrap();
    let err = env.preview(&package).await.unwrap_err();
    assert!(err.to_string().contains("fuera de la carpeta"), "{err}");
}

#[tokio::test]
async fn epics_update_renumber_and_hash_is_pinned() {
    let env = Env::new().await;
    let work = tempfile::tempdir().unwrap();
    let project = env.project("P").await;
    let epic = create_epic(
        &env.store,
        "ui",
        &CreateEpicInput {
            request_id: rid(),
            project_id: project.id.clone(),
            title: "Golden".into(),
            description: Some("d".into()),
        },
    )
    .await
    .unwrap();
    env.story(&project.id, "Golden", "d", &["c"]).await;
    let package = env.export(&project.id, work.path()).await;

    // El hash forma parte del formato v1: si cambia, hay que subir la versión.
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(package.join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["epics"][0]["contentHash"], GOLDEN_EPIC_HASH);
    assert_eq!(manifest["stories"][0]["contentHash"], GOLDEN_STORY_HASH);

    // Épica editada en el paquete → se actualiza.
    let epic_file = package.join(manifest["epics"][0]["file"].as_str().unwrap());
    edit(&epic_file, "title = \"Golden\"", "title = \"Golden 2\"");
    // Épica nueva con un número ya usado aquí → se renumera.
    let new_epic = "0199aaaa-0000-7000-8000-0000000000ee";
    let text = std::fs::read_to_string(&epic_file)
        .unwrap()
        .replace(&epic.id, new_epic)
        .replace("Golden 2", "Nueva")
        .replace("number = 1", "number = 2");
    // E-2 ya existe aquí (creada tras exportar), no en el paquete.
    create_epic(
        &env.store,
        "ui",
        &CreateEpicInput {
            request_id: rid(),
            project_id: project.id.clone(),
            title: "Local".into(),
            description: None,
        },
    )
    .await
    .unwrap();
    std::fs::write(package.join("epics/E-1-nueva.md"), text).unwrap();
    edit(
        &package.join("manifest.json"),
        "\"epics\": [",
        &format!(
            "\"epics\": [\n    {{\"id\": \"{new_epic}\", \"file\": \"epics/E-1-nueva.md\", \
             \"revision\": 1, \"contentHash\": \"\"}},"
        ),
    );
    let preview = env.preview(&package).await.unwrap();
    assert_eq!(
        preview
            .items
            .iter()
            .find(|i| i.id == epic.id)
            .unwrap()
            .action,
        ImportAction::Update
    );
    assert!(
        preview
            .warnings
            .iter()
            .any(|w| w.contains("E-2 «Nueva» se importará como E-3")),
        "{:?}",
        preview.warnings
    );
    env.apply(&package, vec![]).await.unwrap();
    let epics = kanbania_core::features::epics::list_epics(&env.store, &project.id)
        .await
        .unwrap();
    assert_eq!(epics.len(), 3);
    assert!(epics.iter().any(|e| e.title == "Golden 2" && e.number == 1));
    assert!(epics.iter().any(|e| e.title == "Nueva" && e.number == 3));
}

const GOLDEN_EPIC_HASH: &str = "7ec1b80e9d19130c848f3191cce1b787edae598258c405a3672c541bff806287";
const GOLDEN_STORY_HASH: &str = "312c8720496e6b1080ea77682b3bc408f5323d5f54991c135a8ecb2d79b513e3";
