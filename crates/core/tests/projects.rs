//! Pruebas de HU-003 a través de la API pública del núcleo, con SQLite temporal real y
//! repositorios Git temporales (estructura `.git/HEAD`, sin depender del binario `git`).

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kanbania_core::error::{CoreError, ErrorCode};
use kanbania_core::features::projects::{
    CreateProjectInput, Project, RenameProjectInput, create_project, last_opened_project,
    list_projects, open_project, rename_project,
};
use kanbania_core::features::repositories::{
    LinkRepositoryInput, RelocateRepositoryInput, RepositoryLink, RepositoryStatus,
    UnlinkRepositoryInput, link_repository, list_repositories, relocate_repository,
    unlink_repository,
};
use kanbania_core::platform::storage::Store;

const ACTOR: &str = "ui";

struct Env {
    _data: tempfile::TempDir,
    workspace: tempfile::TempDir,
    store: Store,
}

impl Env {
    async fn new() -> Self {
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).await.unwrap();
        Self {
            _data: data,
            workspace: tempfile::tempdir().unwrap(),
            store,
        }
    }

    fn repo(&self, name: &str) -> PathBuf {
        let root = self.workspace.path().join(name);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(root.join("README.md"), format!("# {name}\n")).unwrap();
        root
    }

    async fn project(&self, name: &str) -> Project {
        create_project(&self.store, ACTOR, &create_input(name))
            .await
            .unwrap()
    }

    async fn link(&self, project: &Project, path: &Path) -> Result<RepositoryLink, CoreError> {
        link_repository(
            &self.store,
            ACTOR,
            &LinkRepositoryInput {
                request_id: new_request_id(),
                project_id: project.id.clone(),
                path: path.to_str().unwrap().to_owned(),
            },
        )
        .await
    }
}

fn new_request_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn create_input(name: &str) -> CreateProjectInput {
    CreateProjectInput {
        request_id: new_request_id(),
        name: name.to_owned(),
    }
}

/// Instantánea de contenido + mtime de todos los archivos, para probar que no se modifican.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = std::fs::metadata(&path).unwrap();
            if meta.is_dir() {
                stack.push(path);
            } else {
                out.insert(
                    path.clone(),
                    (std::fs::read(&path).unwrap(), meta.modified().unwrap()),
                );
            }
        }
    }
    out
}

// ---------- CA-1: crear, renombrar, abrir; vincular varios repos sin modificarlos ----------

#[tokio::test]
async fn create_rename_open_and_reopen_after_restart() {
    let data = tempfile::tempdir().unwrap();
    let project_id = {
        let store = Store::open(data.path()).await.unwrap();
        let created = create_project(&store, ACTOR, &create_input("  Kanban Ñandú 🚀 "))
            .await
            .unwrap();
        assert_eq!(created.name, "Kanban Ñandú 🚀");
        assert_eq!(created.revision, 1);
        assert!(last_opened_project(&store).await.unwrap().is_none());

        let renamed = rename_project(
            &store,
            &RenameProjectInput {
                id: created.id.clone(),
                expected_revision: 1,
                name: "Plataforma".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(renamed.revision, 2);

        let opened = open_project(&store, &created.id).await.unwrap();
        assert!(opened.last_opened_at.is_some());
        assert_eq!(opened.revision, 2, "abrir no es una edición");
        store.close().await;
        created.id
    };

    let store = Store::open(data.path()).await.unwrap();
    let last = last_opened_project(&store).await.unwrap().unwrap();
    assert_eq!(last.id, project_id);
    assert_eq!(last.name, "Plataforma");
}

#[tokio::test]
async fn list_orders_by_most_recently_opened() {
    let env = Env::new().await;
    let a = env.project("Alfa").await;
    let b = env.project("Beta").await;
    let _c = env.project("Gamma").await;
    open_project(&env.store, &a.id).await.unwrap();
    open_project(&env.store, &b.id).await.unwrap();

    let names: Vec<String> = list_projects(&env.store)
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["Beta", "Alfa", "Gamma"]);
}

#[tokio::test]
async fn create_project_is_idempotent_and_validates_name() {
    let env = Env::new().await;
    let input = create_input("Una vez");
    let first = create_project(&env.store, ACTOR, &input).await.unwrap();
    let retry = create_project(&env.store, ACTOR, &input).await.unwrap();
    assert_eq!(first, retry);
    assert_eq!(list_projects(&env.store).await.unwrap().len(), 1);

    let invalid = create_project(&env.store, ACTOR, &create_input("   "))
        .await
        .unwrap_err()
        .to_payload(None);
    assert_eq!(invalid.code, ErrorCode::Validation);
    assert_eq!(invalid.field.as_deref(), Some("name"));
}

#[tokio::test]
async fn stale_rename_returns_conflict_with_current_project() {
    let env = Env::new().await;
    let project = env.project("Original").await;
    let rename = |name: &str| RenameProjectInput {
        id: project.id.clone(),
        expected_revision: 1,
        name: name.into(),
    };
    rename_project(&env.store, &rename("Primera"))
        .await
        .unwrap();

    let payload = rename_project(&env.store, &rename("Obsoleta"))
        .await
        .unwrap_err()
        .to_payload(None);
    assert_eq!(payload.code, ErrorCode::Conflict);
    assert_eq!(payload.current.unwrap()["name"], "Primera");
}

#[tokio::test]
async fn links_several_repositories_without_modifying_them() {
    let env = Env::new().await;
    let project = env.project("Multi").await;
    let web = env.repo("web app");
    let api = env.repo("API ñandú 🚀");
    let before = (snapshot(&web), snapshot(&api));

    env.link(&project, &web).await.unwrap();
    env.link(&project, &api).await.unwrap();
    let repos = list_repositories(&env.store, &project.id).await.unwrap();

    assert_eq!(repos.len(), 2);
    assert!(
        repos
            .iter()
            .all(|r| r.status == RepositoryStatus::Available)
    );
    let mut names: Vec<_> = repos.iter().map(|r| r.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["API ñandú 🚀", "web app"]);
    assert_eq!(
        (snapshot(&web), snapshot(&api)),
        before,
        "los repos no deben cambiar"
    );
    assert_eq!(
        list_projects(&env.store).await.unwrap()[0].repository_count,
        2
    );
}

#[tokio::test]
async fn link_to_unknown_project_is_not_found() {
    let env = Env::new().await;
    let repo = env.repo("huérfano");
    let error = link_repository(
        &env.store,
        ACTOR,
        &LinkRepositoryInput {
            request_id: new_request_id(),
            project_id: "no-existe".into(),
            path: repo.to_str().unwrap().into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
}

// ---------- CA-2: ruta canónica, repo Git, duplicados, espacios y Unicode ----------

#[tokio::test]
async fn rejects_paths_that_are_not_repository_roots() {
    let env = Env::new().await;
    let project = env.project("Validación").await;
    let plain = env.workspace.path().join("sin git");
    std::fs::create_dir_all(&plain).unwrap();
    let repo = env.repo("repo");

    for path in [
        plain.clone(),
        repo.join(".git"),
        env.workspace.path().join("no existe"),
    ] {
        let payload = env
            .link(&project, &path)
            .await
            .unwrap_err()
            .to_payload(None);
        assert_eq!(payload.code, ErrorCode::Validation, "{path:?}");
        assert_eq!(payload.field.as_deref(), Some("path"));
    }
    assert!(
        list_repositories(&env.store, &project.id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn duplicate_link_is_rejected_by_identity_not_spelling() {
    let env = Env::new().await;
    let project = env.project("Duplicados").await;
    let repo = env.repo("Repo Único");
    env.link(&project, &repo).await.unwrap();

    // Misma ruta y ruta con componentes `..` redundantes.
    let dotted = repo.join("..").join("Repo Único");
    for path in [repo.clone(), dotted] {
        let error = env.link(&project, &path).await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::AlreadyExists, "{path:?}");
    }

    // Otra grafía en sistemas de archivos que no distinguen mayúsculas (APFS, NTFS por defecto).
    let other_case = env.workspace.path().join("REPO ÚNICO");
    if other_case.exists() {
        let error = env.link(&project, &other_case).await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::AlreadyExists);
    }

    assert_eq!(
        list_repositories(&env.store, &project.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_policy_stores_target_and_detects_duplicate() {
    let env = Env::new().await;
    let project = env.project("Enlaces").await;
    let repo = env.repo("destino");
    let link = env.workspace.path().join("acceso directo");
    std::os::unix::fs::symlink(&repo, &link).unwrap();

    let linked = env.link(&project, &link).await.unwrap();
    assert_eq!(
        Path::new(&linked.path),
        dunce::canonicalize(&repo).unwrap(),
        "se guarda la ruta real, no el enlace"
    );

    let error = env.link(&project, &repo).await.unwrap_err();
    assert_eq!(error.code(), ErrorCode::AlreadyExists);
}

#[tokio::test]
async fn same_repository_may_belong_to_several_projects() {
    let env = Env::new().await;
    let shared = env.repo("compartida");
    let web = env.project("Web").await;
    let api = env.project("API").await;

    env.link(&web, &shared).await.unwrap();
    env.link(&api, &shared).await.unwrap();
}

// ---------- CA-3: repo movido o inaccesible → reubicar o desvincular sin borrar nada ----------

#[tokio::test]
async fn moved_repository_can_be_relocated() {
    let env = Env::new().await;
    let project = env.project("Mudanza").await;
    let original = env.repo("antes");
    let linked = env.link(&project, &original).await.unwrap();

    let moved = env.workspace.path().join("después ñ");
    std::fs::rename(&original, &moved).unwrap();
    let repos = list_repositories(&env.store, &project.id).await.unwrap();
    assert_eq!(repos[0].status, RepositoryStatus::Missing);

    let relocated = relocate_repository(
        &env.store,
        &RelocateRepositoryInput {
            project_id: project.id.clone(),
            id: linked.id.clone(),
            expected_revision: linked.revision,
            path: moved.to_str().unwrap().into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(relocated.status, RepositoryStatus::Available);
    assert_eq!(relocated.revision, linked.revision + 1);
    assert_eq!(relocated.name, "después ñ");
}

#[tokio::test]
async fn relocate_validates_target_and_duplicates() {
    let env = Env::new().await;
    let project = env.project("Reubicar").await;
    let a = env.link(&project, &env.repo("a")).await.unwrap();
    let b_path = env.repo("b");
    env.link(&project, &b_path).await.unwrap();
    let relocate = |path: &Path, revision: i64| RelocateRepositoryInput {
        project_id: project.id.clone(),
        id: a.id.clone(),
        expected_revision: revision,
        path: path.to_str().unwrap().into(),
    };

    let duplicate = relocate_repository(&env.store, &relocate(&b_path, 1))
        .await
        .unwrap_err();
    assert_eq!(duplicate.code(), ErrorCode::AlreadyExists);

    let plain = env.workspace.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let invalid = relocate_repository(&env.store, &relocate(&plain, 1))
        .await
        .unwrap_err();
    assert_eq!(invalid.code(), ErrorCode::Validation);

    let stale = relocate_repository(&env.store, &relocate(&env.repo("c"), 7))
        .await
        .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::Conflict);
}

#[tokio::test]
async fn unlink_removes_only_the_link() {
    let env = Env::new().await;
    let project = env.project("Desvincular").await;
    let repo = env.repo("conservar");
    let linked = env.link(&project, &repo).await.unwrap();
    let before = snapshot(&repo);

    let stale = unlink_repository(
        &env.store,
        &UnlinkRepositoryInput {
            project_id: project.id.clone(),
            id: linked.id.clone(),
            expected_revision: 99,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::Conflict);

    unlink_repository(
        &env.store,
        &UnlinkRepositoryInput {
            project_id: project.id.clone(),
            id: linked.id.clone(),
            expected_revision: linked.revision,
        },
    )
    .await
    .unwrap();

    assert!(
        list_repositories(&env.store, &project.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        snapshot(&repo),
        before,
        "los archivos del repo siguen intactos"
    );
    assert_eq!(list_projects(&env.store).await.unwrap().len(), 1);
}

#[tokio::test]
async fn missing_repository_can_be_unlinked() {
    let env = Env::new().await;
    let project = env.project("Perdido").await;
    let repo = env.repo("borrado");
    let linked = env.link(&project, &repo).await.unwrap();
    std::fs::remove_dir_all(&repo).unwrap();

    unlink_repository(
        &env.store,
        &UnlinkRepositoryInput {
            project_id: project.id.clone(),
            id: linked.id,
            expected_revision: linked.revision,
        },
    )
    .await
    .unwrap();
    assert!(
        list_repositories(&env.store, &project.id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn unreadable_repository_is_reported_inaccessible() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new().await;
    let project = env.project("Permisos").await;
    let parent = env.workspace.path().join("cerrado");
    std::fs::create_dir_all(&parent).unwrap();
    let repo = {
        let root = parent.join("repo");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        root
    };
    env.link(&project, &repo).await.unwrap();

    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o000)).unwrap();
    let status = list_repositories(&env.store, &project.id).await.unwrap()[0].status;
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(status, RepositoryStatus::Inaccessible);
}

#[tokio::test]
async fn relocate_and_unlink_are_scoped_to_their_project() {
    let env = Env::new().await;
    let owner = env.project("Dueño").await;
    let other = env.project("Ajeno").await;
    let linked = env.link(&owner, &env.repo("privado")).await.unwrap();

    let relocate = relocate_repository(
        &env.store,
        &RelocateRepositoryInput {
            project_id: other.id.clone(),
            id: linked.id.clone(),
            expected_revision: linked.revision,
            path: env.repo("otro").to_str().unwrap().into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(relocate.code(), ErrorCode::NotFound);

    let unlink = unlink_repository(
        &env.store,
        &UnlinkRepositoryInput {
            project_id: other.id.clone(),
            id: linked.id.clone(),
            expected_revision: linked.revision,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(unlink.code(), ErrorCode::NotFound);
    assert_eq!(
        list_repositories(&env.store, &owner.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn link_retry_returns_original_result_even_after_repository_moved() {
    let env = Env::new().await;
    let project = env.project("Reintento").await;
    let repo = env.repo("se moverá");
    let input = LinkRepositoryInput {
        request_id: new_request_id(),
        project_id: project.id.clone(),
        path: repo.to_str().unwrap().into(),
    };
    let first = link_repository(&env.store, ACTOR, &input).await.unwrap();

    std::fs::rename(&repo, env.workspace.path().join("ya no está")).unwrap();
    let retry = link_repository(&env.store, ACTOR, &input).await.unwrap();

    assert_eq!(retry.id, first.id);
    assert_eq!(
        retry.status,
        RepositoryStatus::Missing,
        "estado recalculado al responder"
    );
    assert_eq!(
        list_repositories(&env.store, &project.id)
            .await
            .unwrap()
            .len(),
        1
    );
}
