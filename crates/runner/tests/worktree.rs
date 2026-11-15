//! Worktrees con `git` real en carpetas temporales con espacios.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use kanbania_runner::worktree::{
    CreateWorktree, GitConfig, RemoveOutcome, WorktreeErrorKind, create_worktree, remove_worktree,
    worktree_status,
};

struct Fixture {
    _dir: tempfile::TempDir,
    base: PathBuf,
    repo: PathBuf,
    root: PathBuf,
    git: GitConfig,
    commit: String,
}

/// Git para preparar el escenario (no el del runner): identidad fija y sin firma.
fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "user.name=Prueba",
            "-c",
            "user.email=prueba@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = dunce::canonicalize(dir.path()).unwrap();
    let repo = base.join("repositorio con espacios");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("seguido.txt"), "original\n").unwrap();
    std::fs::write(repo.join("otro.txt"), "otro\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "inicial"]);
    let commit = git(&repo, &["rev-parse", "HEAD"]).trim().to_owned();
    let data = base
        .join("Application Support")
        .join("dev.ikkidev.kanbania");
    let git_config = GitConfig::new(data.join("no-hooks"));
    Fixture {
        _dir: dir,
        base,
        repo,
        root: data.join("worktrees"),
        git: git_config,
        commit,
    }
}

fn request(f: &Fixture, run: &str, branch: &str) -> CreateWorktree {
    CreateWorktree {
        repository: f.repo.clone(),
        root: f.root.clone(),
        project_segment: "proyecto-1".into(),
        run_segment: run.into(),
        branch: branch.into(),
        commit: f.commit.clone(),
    }
}

/// Estado observable del repositorio original: rama, HEAD, `status` y contenido de archivos.
fn snapshot(repo: &Path) -> (String, String, String, String, String) {
    (
        git(repo, &["symbolic-ref", "HEAD"]),
        git(repo, &["rev-parse", "HEAD"]),
        git(repo, &["status", "--porcelain=v1", "--untracked-files=all"]),
        std::fs::read_to_string(repo.join("seguido.txt")).unwrap(),
        std::fs::read_to_string(repo.join("sin seguimiento.txt")).unwrap_or_default(),
    )
}

#[tokio::test]
async fn dirty_original_repository_is_untouched_by_create_and_remove() {
    let f = fixture();
    // Cambios sin commit: uno preparado, uno sin preparar y un archivo sin seguimiento.
    std::fs::write(f.repo.join("seguido.txt"), "cambio local sin commit\n").unwrap();
    std::fs::write(f.repo.join("otro.txt"), "preparado\n").unwrap();
    git(&f.repo, &["add", "otro.txt"]);
    std::fs::write(f.repo.join("sin seguimiento.txt"), "nuevo\n").unwrap();
    let before = snapshot(&f.repo);

    let created = create_worktree(&f.git, &request(&f, "run-1", "kanbania/h1-i1-0123abcd"))
        .await
        .unwrap();
    assert_eq!(
        created.path,
        dunce::canonicalize(&f.root)
            .unwrap()
            .join("proyecto-1")
            .join("run-1")
    );
    assert!(
        created
            .path
            .to_str()
            .unwrap()
            .contains("Application Support")
    );
    // El worktree parte del commit, sin los cambios locales del original.
    assert_eq!(
        std::fs::read_to_string(created.path.join("seguido.txt")).unwrap(),
        "original\n"
    );
    assert!(!created.path.join("sin seguimiento.txt").exists());
    assert_eq!(git(&created.path, &["rev-parse", "HEAD"]).trim(), f.commit);
    assert_eq!(
        git(&created.path, &["symbolic-ref", "HEAD"]).trim(),
        "refs/heads/kanbania/h1-i1-0123abcd"
    );
    assert_eq!(snapshot(&f.repo), before);

    let status = worktree_status(&f.git, &f.repo, &f.root, &created.path, &created.branch)
        .await
        .unwrap();
    assert!(status.is_clean());
    let removed = remove_worktree(
        &f.git,
        &f.repo,
        &f.root,
        &created.path,
        &created.branch,
        false,
    )
    .await
    .unwrap();
    assert_eq!(removed, RemoveOutcome::Removed);
    assert!(!created.path.exists());
    assert_eq!(snapshot(&f.repo), before);
    // La rama se conserva.
    git(
        &f.repo,
        &["show-ref", "--verify", "refs/heads/kanbania/h1-i1-0123abcd"],
    );
}

#[tokio::test]
async fn branch_and_path_are_never_reused() {
    let f = fixture();
    create_worktree(&f.git, &request(&f, "run-1", "kanbania/unica"))
        .await
        .unwrap();
    let error = create_worktree(&f.git, &request(&f, "run-2", "kanbania/unica"))
        .await
        .unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::BranchExists);
    assert!(!f.root.join("proyecto-1").join("run-2").exists());

    let error = create_worktree(&f.git, &request(&f, "run-1", "kanbania/otra"))
        .await
        .unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::PathExists);
    // Ningún mensaje visible lleva rutas.
    assert!(!error.to_string().contains(f.base.to_str().unwrap()));
}

#[tokio::test]
async fn unknown_commit_and_invalid_input_are_rejected() {
    let f = fixture();
    let mut unknown = request(&f, "run-1", "kanbania/x");
    unknown.commit = "0".repeat(40);
    let error = create_worktree(&f.git, &unknown).await.unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::CommitNotFound);

    let mut traversal = request(&f, "..", "kanbania/x");
    traversal.run_segment = "..".into();
    let error = create_worktree(&f.git, &traversal).await.unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::InvalidInput);

    let error = create_worktree(&f.git, &request(&f, "run-1", "-x"))
        .await
        .unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::InvalidInput);

    let mut not_repo = request(&f, "run-1", "kanbania/x");
    not_repo.repository = f.base.clone();
    let error = create_worktree(&f.git, &not_repo).await.unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::NotARepository);
}

#[tokio::test]
async fn worktree_with_changes_needs_explicit_force() {
    let f = fixture();
    let created = create_worktree(&f.git, &request(&f, "run-1", "kanbania/cambios"))
        .await
        .unwrap();
    std::fs::write(created.path.join("seguido.txt"), "modificado\n").unwrap();
    std::fs::write(created.path.join("nuevo con espacio.txt"), "x\n").unwrap();
    std::fs::create_dir(created.path.join("sub dir")).unwrap();
    std::fs::write(created.path.join("sub dir").join("y.txt"), "y\n").unwrap();

    let status = worktree_status(&f.git, &f.repo, &f.root, &created.path, &created.branch)
        .await
        .unwrap();
    assert_eq!(status.changes, 3);
    assert!(status.complete);

    let error = remove_worktree(
        &f.git,
        &f.repo,
        &f.root,
        &created.path,
        &created.branch,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::HasChanges { changes: 3 });
    assert!(created.path.join("nuevo con espacio.txt").exists());

    let removed = remove_worktree(
        &f.git,
        &f.repo,
        &f.root,
        &created.path,
        &created.branch,
        true,
    )
    .await
    .unwrap();
    assert_eq!(removed, RemoveOutcome::Forced);
    assert!(!created.path.exists());
    git(
        &f.repo,
        &["show-ref", "--verify", "refs/heads/kanbania/cambios"],
    );

    // Ya no existe: no se toca nada.
    let again = remove_worktree(
        &f.git,
        &f.repo,
        &f.root,
        &created.path,
        &created.branch,
        true,
    )
    .await
    .unwrap();
    assert_eq!(again, RemoveOutcome::Missing);
}

#[tokio::test]
async fn only_registered_worktrees_under_the_root_are_removed() {
    let f = fixture();
    // Carpeta con la forma correcta pero no registrada.
    let fake = f.root.join("proyecto-1").join("run-falso");
    std::fs::create_dir_all(&fake).unwrap();
    std::fs::write(fake.join("dato.txt"), "no borrar\n").unwrap();
    let error = remove_worktree(&f.git, &f.repo, &f.root, &fake, "kanbania/x", true)
        .await
        .unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::NotRegistered);
    assert!(fake.join("dato.txt").exists());

    // El repositorio principal, fuera de la raíz.
    let error = remove_worktree(&f.git, &f.repo, &f.root, &f.repo, "kanbania/x", true)
        .await
        .unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::NotRegistered);
    assert!(f.repo.join("seguido.txt").exists());

    // Un worktree registrado pero fuera de la raíz (creado a mano).
    let outside = f.base.join("fuera de la raiz");
    git(
        &f.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "manual",
            outside.to_str().unwrap(),
            &f.commit,
        ],
    );
    let error = remove_worktree(&f.git, &f.repo, &f.root, &outside, "kanbania/x", true)
        .await
        .unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::NotRegistered);
    assert!(outside.join("seguido.txt").exists());

    // Un nivel por encima del intento (la carpeta del proyecto).
    create_worktree(&f.git, &request(&f, "run-1", "kanbania/nivel"))
        .await
        .unwrap();
    let error = remove_worktree(
        &f.git,
        &f.repo,
        &f.root,
        &f.root.join("proyecto-1"),
        "kanbania/x",
        true,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::NotRegistered);
    assert!(f.root.join("proyecto-1").join("run-1").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn hooks_are_not_executed() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    let witness = f.base.join("testigo del hook");
    let hook = f.repo.join(".git").join("hooks").join("post-checkout");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\ntouch '{}'\n", witness.to_str().unwrap()),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    // Comprobación del escenario: con `git` normal, `worktree add` sí ejecuta el hook.
    let control = f.base.join("control");
    git(
        &f.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "control",
            control.to_str().unwrap(),
            &f.commit,
        ],
    );
    assert!(witness.exists(), "el hook de control no se ejecutó");
    std::fs::remove_file(&witness).unwrap();

    create_worktree(&f.git, &request(&f, "run-1", "kanbania/sin-hooks"))
        .await
        .unwrap();
    assert!(!witness.exists(), "se ejecutó el hook post-checkout");
}

#[tokio::test]
async fn commits_outside_any_branch_or_another_branch_need_force() {
    let f = fixture();
    // HEAD separado con un commit nuevo que no está en ninguna rama.
    let detached = create_worktree(&f.git, &request(&f, "run-1", "kanbania/separada"))
        .await
        .unwrap();
    git(&detached.path, &["checkout", "-q", "--detach"]);
    std::fs::write(detached.path.join("nuevo.txt"), "x\n").unwrap();
    git(&detached.path, &["add", "."]);
    git(&detached.path, &["commit", "-q", "-m", "solo en HEAD"]);
    let status = worktree_status(&f.git, &f.repo, &f.root, &detached.path, &detached.branch)
        .await
        .unwrap();
    assert_eq!(status.changes, 0);
    assert!(status.commits_at_risk);
    let error = remove_worktree(
        &f.git,
        &f.repo,
        &f.root,
        &detached.path,
        &detached.branch,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::CommitsAtRisk);
    assert!(error.needs_force());
    assert!(detached.path.join("nuevo.txt").exists());

    // En otra rama distinta de la del intento (aunque esté limpio).
    let other = create_worktree(&f.git, &request(&f, "run-2", "kanbania/otra-rama"))
        .await
        .unwrap();
    git(&other.path, &["checkout", "-q", "-b", "cambiada"]);
    let error = remove_worktree(&f.git, &f.repo, &f.root, &other.path, &other.branch, false)
        .await
        .unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::CommitsAtRisk);
    assert!(other.path.exists());

    // Forzar (acción explícita) sí elimina.
    let removed = remove_worktree(
        &f.git,
        &f.repo,
        &f.root,
        &detached.path,
        &detached.branch,
        true,
    )
    .await
    .unwrap();
    assert_eq!(removed, RemoveOutcome::Forced);
}

#[tokio::test]
async fn ignored_files_require_force_to_remove() {
    let f = fixture();
    std::fs::write(f.repo.join(".gitignore"), "*.log\n").unwrap();
    git(&f.repo, &["add", ".gitignore"]);
    git(&f.repo, &["commit", "-q", "-m", "ignorar logs"]);
    let mut req = request(&f, "run-1", "kanbania/ignorados");
    req.commit = git(&f.repo, &["rev-parse", "HEAD"]).trim().to_owned();
    let created = create_worktree(&f.git, &req).await.unwrap();
    std::fs::write(created.path.join("salida.log"), "datos\n").unwrap();
    let status = worktree_status(&f.git, &f.repo, &f.root, &created.path, &created.branch)
        .await
        .unwrap();
    assert_eq!((status.changes, status.ignored), (0, 1));
    assert!(!status.is_clean());
    // Git los borraría sin forzar: la decisión del propietario es exigir descartarlos.
    let error = remove_worktree(
        &f.git,
        &f.repo,
        &f.root,
        &created.path,
        &created.branch,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::HasIgnored { ignored: 1 });
    assert!(error.needs_force());
    assert!(created.path.join("salida.log").exists());
    let removed = remove_worktree(
        &f.git,
        &f.repo,
        &f.root,
        &created.path,
        &created.branch,
        true,
    )
    .await
    .unwrap();
    assert_eq!(removed, RemoveOutcome::Forced);
    assert!(!created.path.exists());
    // La rama se conserva.
    git(
        &f.repo,
        &["rev-parse", "--verify", "refs/heads/kanbania/ignorados"],
    );
}

#[tokio::test]
async fn a_folder_inside_another_repository_is_not_a_repository() {
    let f = fixture();
    let inner = f.repo.join("subcarpeta");
    std::fs::create_dir(&inner).unwrap();
    let mut req = request(&f, "run-1", "kanbania/x");
    req.repository = inner;
    let error = create_worktree(&f.git, &req).await.unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::NotARepository);
    assert!(!error.add_attempted);
    assert!(!f.root.join("proyecto-1").join("run-1").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_project_or_run_folders_are_never_used_or_removed() {
    use std::os::unix::fs::symlink;
    let f = fixture();
    // Carpeta del proyecto sustituida por un enlace a otro sitio: no se crea nada ahí.
    let elsewhere = f.base.join("otro sitio");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::create_dir_all(&f.root).unwrap();
    symlink(&elsewhere, f.root.join("proyecto-enlazado")).unwrap();
    let mut req = request(&f, "run-1", "kanbania/enlace");
    req.project_segment = "proyecto-enlazado".into();
    let error = create_worktree(&f.git, &req).await.unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::InvalidInput);
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);

    // Carpeta del intento sustituida por un enlace a otro worktree registrado.
    let target = create_worktree(&f.git, &request(&f, "run-a", "kanbania/destino"))
        .await
        .unwrap();
    let victim = create_worktree(&f.git, &request(&f, "run-b", "kanbania/victima"))
        .await
        .unwrap();
    std::fs::rename(&victim.path, f.base.join("victima movida")).unwrap();
    symlink(&target.path, &victim.path).unwrap();
    let error = remove_worktree(&f.git, &f.repo, &f.root, &victim.path, &victim.branch, true)
        .await
        .unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::NotRegistered);
    assert!(target.path.join("seguido.txt").exists());

    // … o a una carpeta cualquiera con datos.
    let data = f.base.join("datos valiosos");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("importante.txt"), "no borrar\n").unwrap();
    std::fs::remove_file(&victim.path).unwrap();
    symlink(&data, &victim.path).unwrap();
    let error = remove_worktree(&f.git, &f.repo, &f.root, &victim.path, &victim.branch, true)
        .await
        .unwrap_err();
    assert_eq!(error.kind, WorktreeErrorKind::NotRegistered);
    assert!(data.join("importante.txt").exists());
    let status = worktree_status(&f.git, &f.repo, &f.root, &victim.path, &victim.branch).await;
    assert_eq!(status.unwrap_err().kind, WorktreeErrorKind::NotRegistered);
}
