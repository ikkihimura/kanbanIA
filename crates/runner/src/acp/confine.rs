//! Aislamiento de los datos de la app frente a los agentes (HU-023, decisión del propietario:
//! sandbox por agente). El agente no puede leer ni escribir `<data_dir>` (base, copias,
//! `service.json`, la salida de otros intentos ni otros worktrees) salvo el worktree de su
//! intento:
//!
//! - **Claude Code** (macOS): el adaptador se lanza con `sandbox-exec` y
//!   [`DATA_DIR_SANDBOX_PROFILE`]. Lo aplica el sistema al proceso y a todos sus descendientes
//!   (herramienta Read, `cat`, lo que sea). Se niega el **contenido** (`file-read-data`) y no los
//!   metadatos: Node resuelve el `cwd` con `lstat` de cada carpeta padre (comprobado: negar
//!   `file-read*` impide arrancar). En Windows y Linux no hay mecanismo: se avisa.
//! - **Codex**: macOS no permite anidar sandboxes (`sandbox_apply: Operation not permitted`,
//!   comprobado) y Codex ejecuta cada comando dentro del suyo, así que no se puede envolver. Se
//!   le da un perfil de permisos propio ([`codex_config`]) que aplica su sandbox a cada comando
//!   (según su documentación, también en Windows: sin comprobar). No se niega ninguna carpeta que contenga el worktree: Codex solo le
//!   describe al modelo lo denegado y, con «negar `<datos>`, leer `<worktree>`», el modelo
//!   concluía que su carpeta tampoco era legible y no lo intentaba (prueba real, 2026-11-17).
//!   Se niega cada entrada **hermana** del camino hasta el worktree
//!   ([`DataConfinement::codex_denied`]). El propio proceso de Codex (no sus comandos) sigue
//!   leyendo su `codex-home`.
//!
//! Todo con rutas canónicas: el sandbox compara rutas reales (`/tmp` es `/private/tmp`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Programa del sandbox de macOS (parte del sistema).
pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Perfil SBPL: todo permitido salvo leer el contenido o escribir en `DATA_DIR`, excepto en
/// `WORKTREE` (más específico, va después). Las rutas van como parámetros (`-D`), nunca dentro
/// del texto del perfil: no hay que escapar nada.
///
/// Además niega ejecutar `launchctl`, `open` y `osascript`: piden a launchd o a
/// LaunchServices que lancen procesos que **no** heredan el sandbox. Es una mitigación parcial
/// (una copia de esos programas o hablar XPC directamente no se impide): un comando que la
/// persona apruebe podría lanzar procesos fuera del sandbox.
pub const DATA_DIR_SANDBOX_PROFILE: &str = "(version 1)(allow default)\
(deny file-read-data file-write* (subpath (param \"DATA_DIR\")))\
(allow file-read-data file-write* (subpath (param \"WORKTREE\")))\
(deny process-exec (literal \"/bin/launchctl\") (literal \"/usr/bin/open\") \
(literal \"/usr/bin/osascript\"))";

/// Carpeta de los worktrees dentro de los datos (la misma que `runs::WORKTREES_DIR`; el
/// cliente ACP no enlaza el núcleo).
const WORKTREES_DIR: &str = "worktrees";

/// Nombre del perfil de permisos que la app define para Codex.
pub const CODEX_PERMISSIONS_PROFILE: &str = "kanbania";

/// Línea `sys` fija: Claude Code dentro del sandbox de macOS.
pub const CLAUDE_SANDBOX_NOTE: &str = "La app ejecuta Claude Code en un sandbox de macOS que le \
     impide leer o escribir directamente los datos de Kanban IA (base, copias, descriptor del \
     servicio, otros intentos) salvo el worktree de este intento. Un comando que apruebes \
     podría lanzar procesos fuera del sandbox.";

/// Línea `sys` fija: plataforma sin sandbox para Claude Code.
pub const CLAUDE_UNCONFINED_NOTE: &str = "En esta plataforma la app no puede impedir que Claude \
     Code lea los datos de Kanban IA (base, copias, descriptor del servicio).";

/// Línea `sys` fija: perfil de permisos de Codex.
pub const CODEX_CONFINEMENT_NOTE: &str = "La app da a Codex un perfil de permisos que niega a sus \
     comandos los datos de Kanban IA (base, copias, descriptor del servicio, su carpeta de \
     Codex y los intentos que ya existían), salvo el worktree de este intento. Lo aplica el \
     sandbox de Codex; un comando que apruebes para salir del sandbox no queda limitado.";

/// Rutas canónicas de los datos de la app y del worktree del intento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataConfinement {
    data_dir: String,
    worktree: String,
    codex_denied: Vec<String>,
}

/// Entradas de la raíz de los datos que se niegan a Codex aunque aún no existan (Codex omite
/// las que faltan; con estas, la lista no depende de que ya se hayan creado).
const KNOWN_DATA_ENTRIES: &[&str] = &[
    "kanbania.sqlite3",
    "kanbania.sqlite3-wal",
    "kanbania.sqlite3-shm",
    "kanbania.lock",
    "service.json",
    "service.json.tmp",
    "kanbania.sqlite3-journal",
    "kanbania.sqlite3.restoring",
    "backups",
    "runs",
    "codex-home",
];

impl DataConfinement {
    /// Canonicaliza ambas rutas. Falla (quien llama no lanza nada) si no existen, no son UTF-8
    /// (van en un parámetro del perfil y en JSON) o si el worktree contiene los datos (el
    /// permiso del worktree abriría todo). E/S bloqueante y breve.
    pub fn new(data_dir: &Path, worktree: &Path) -> std::io::Result<Self> {
        // `dunce`: en Windows, sin el prefijo `\\?\` que no coincidiría con las rutas que
        // compara el sandbox de Codex.
        let data_dir = dunce::canonicalize(data_dir)?;
        let worktree = dunce::canonicalize(worktree)?;
        let invalid =
            |reason: &str| std::io::Error::new(std::io::ErrorKind::InvalidInput, reason.to_owned());
        if data_dir.starts_with(&worktree) {
            return Err(invalid("el worktree contiene los datos de la app"));
        }
        // Dentro de los datos, solo `worktrees/<proyecto>/<intento>` ya resuelto: un worktree
        // convertido en enlace (p. ej. a `backups`) abriría con su permiso otra carpeta.
        if let Ok(relative) = worktree.strip_prefix(&data_dir) {
            let parts: Vec<_> = relative.components().collect();
            if parts.len() != 3 || parts[0].as_os_str() != WORKTREES_DIR {
                return Err(invalid("el worktree no está en la carpeta de worktrees"));
            }
        }
        let codex_denied = denied_siblings(&data_dir, &worktree)?;
        Ok(Self {
            data_dir: utf8(data_dir)?,
            worktree: utf8(worktree)?,
            codex_denied,
        })
    }

    /// Rutas que se niegan a Codex: con el worktree dentro de los datos, todas las entradas
    /// hermanas de cada carpeta del camino (la base, el descriptor, las copias, la salida de los
    /// intentos, su carpeta de Codex, otros proyectos y otros intentos) más
    /// [`KNOWN_DATA_ENTRIES`]; si no, los datos enteros. Lo que se cree después en esas
    /// carpetas (p. ej. el worktree de otro intento que empiece más tarde) no queda negado.
    pub fn codex_denied(&self) -> &[String] {
        &self.codex_denied
    }

    pub fn data_dir(&self) -> &str {
        &self.data_dir
    }

    pub fn worktree(&self) -> &str {
        &self.worktree
    }

    /// Programa y argumentos para lanzar `program args…` dentro de [`DATA_DIR_SANDBOX_PROFILE`]
    /// (`sandbox-exec` sustituye su imagen por la del programa: mismo proceso y grupo).
    pub fn sandboxed(&self, program: &Path, args: &[OsString]) -> (PathBuf, Vec<OsString>) {
        let mut wrapped: Vec<OsString> = vec![
            "-D".into(),
            format!("DATA_DIR={}", self.data_dir).into(),
            "-D".into(),
            format!("WORKTREE={}", self.worktree).into(),
            "-p".into(),
            DATA_DIR_SANDBOX_PROFILE.into(),
            program.as_os_str().to_owned(),
        ];
        wrapped.extend(args.iter().cloned());
        (PathBuf::from(SANDBOX_EXEC), wrapped)
    }
}

fn denied_siblings(data_dir: &Path, worktree: &Path) -> std::io::Result<Vec<String>> {
    let Ok(relative) = worktree.strip_prefix(data_dir) else {
        return Ok(vec![utf8(data_dir.to_path_buf())?]);
    };
    let mut denied: Vec<PathBuf> = KNOWN_DATA_ENTRIES
        .iter()
        .map(|name| data_dir.join(name))
        .collect();
    let mut dir = data_dir.to_path_buf();
    for component in relative.components() {
        let keep = component.as_os_str();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.file_name() != keep {
                denied.push(entry.path());
            }
        }
        dir.push(keep);
    }
    denied.sort();
    denied.dedup();
    denied.into_iter().map(utf8).collect()
}

fn utf8(path: PathBuf) -> std::io::Result<String> {
    path.into_os_string()
        .into_string()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "ruta que no es UTF-8"))
}

/// ¿Hay sandbox del sistema para Claude Code en esta plataforma?
pub const CLAUDE_SANDBOX_AVAILABLE: bool = cfg!(target_os = "macos");

/// [`super::CODEX_CONFIG`] más el perfil de permisos de la app: `extends = ":read-only"` (el
/// mismo modo que fija el adaptador) y [`DataConfinement::codex_denied`] denegadas.
pub fn codex_config(confinement: &DataConfinement) -> String {
    let mut config: serde_json::Value =
        serde_json::from_str(super::CODEX_CONFIG).unwrap_or_else(|_| serde_json::json!({}));
    let filesystem: serde_json::Map<String, serde_json::Value> = confinement
        .codex_denied
        .iter()
        .map(|path| (path.clone(), "deny".into()))
        .collect();
    if let Some(map) = config.as_object_mut() {
        map.insert(
            "default_permissions".into(),
            CODEX_PERMISSIONS_PROFILE.into(),
        );
        map.insert(
            "permissions".into(),
            serde_json::json!({
                CODEX_PERMISSIONS_PROFILE: {
                    "extends": ":read-only",
                    "filesystem": filesystem,
                }
            }),
        );
    }
    config.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("datos");
        let worktree = data.join("worktrees").join("p").join("r");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(data.join("kanbania.sqlite3"), "base").unwrap();
        std::fs::write(worktree.join("a.txt"), "worktree").unwrap();
        (dir, data, worktree)
    }

    #[test]
    fn paths_are_canonical_and_the_worktree_cannot_contain_the_data() {
        let (_dir, data, worktree) = layout();
        let c = DataConfinement::new(&data, &worktree).unwrap();
        assert_eq!(Path::new(c.data_dir()), dunce::canonicalize(&data).unwrap());
        assert!(Path::new(c.worktree()).starts_with(c.data_dir()));
        assert!(DataConfinement::new(&data, &data).is_err());
        assert!(DataConfinement::new(&worktree, &data).is_err());
        assert!(DataConfinement::new(&data, &data.join("no-existe")).is_err());
        // Dentro de los datos, solo `worktrees/<p>/<i>`.
        std::fs::create_dir_all(data.join("backups").join("x")).unwrap();
        assert!(DataConfinement::new(&data, &data.join("backups")).is_err());
        assert!(DataConfinement::new(&data, &data.join("worktrees").join("p")).is_err());
        #[cfg(unix)]
        {
            let link = data.join("worktrees").join("p").join("enlace");
            std::os::unix::fs::symlink(data.join("backups").join("x"), &link).unwrap();
            assert!(DataConfinement::new(&data, &link).is_err());
        }
    }

    #[test]
    fn codex_profile_denies_every_sibling_but_no_ancestor_of_the_worktree() {
        let (_dir, data, worktree) = layout();
        std::fs::create_dir_all(data.join("worktrees").join("p").join("otro")).unwrap();
        std::fs::create_dir_all(data.join("worktrees").join("q")).unwrap();
        std::fs::write(data.join("extra.txt"), "x").unwrap();
        let c = DataConfinement::new(&data, &worktree).unwrap();
        let config: serde_json::Value = serde_json::from_str(&codex_config(&c)).unwrap();
        assert_eq!(config["default_permissions"], "kanbania");
        let profile = &config["permissions"]["kanbania"];
        assert_eq!(profile["extends"], ":read-only");
        let filesystem = profile["filesystem"].as_object().unwrap();
        let root = Path::new(c.data_dir());
        for denied in [
            root.join("kanbania.sqlite3"),
            root.join("service.json"),
            root.join("backups"),
            root.join("codex-home"),
            root.join("extra.txt"),
            root.join("worktrees").join("q"),
            root.join("worktrees").join("p").join("otro"),
        ] {
            assert_eq!(filesystem[denied.to_str().unwrap()], "deny", "{denied:?}");
        }
        for (path, access) in filesystem {
            assert_eq!(access, "deny");
            assert!(
                !Path::new(c.worktree()).starts_with(path),
                "niega un antecesor del worktree: {path}"
            );
        }
        // Fuera de los datos: se niegan enteros.
        let other = tempfile::tempdir().unwrap();
        let outside = DataConfinement::new(&data, other.path()).unwrap();
        assert_eq!(outside.codex_denied(), [c.data_dir().to_owned()]);
        // Lo de siempre sigue igual.
        assert_eq!(config["web_search"], "disabled");
        assert_eq!(config["features"]["remote_plugin"], false);
    }

    /// El perfil real del sistema: sin contenido ni escritura en los datos; el worktree, sí; el
    /// `cwd` se resuelve (metadatos permitidos).
    #[cfg(target_os = "macos")]
    #[test]
    fn the_macos_sandbox_hides_the_data_but_not_the_worktree() {
        let (_dir, data, worktree) = layout();
        let c = DataConfinement::new(&data, &worktree).unwrap();
        let upper = c.data_dir().to_uppercase();
        let script = "cat \"$1/kanbania.sqlite3\" && echo LEE_BASE; ls \"$1\" && echo LISTA; \
                      cat a.txt; echo x > nuevo.txt && echo ESCRIBE_WT; \
                      echo x > \"$1/plantado\" && echo ESCRIBE_DATOS; \
                      cat \"$2/KANBANIA.SQLITE3\" && echo LEE_MAYUSCULAS; \
                      ln -s \"$1/kanbania.sqlite3\" simbolico; cat simbolico && echo LEE_SIMBOLICO; \
                      ln \"$1/kanbania.sqlite3\" duro && echo CREA_DURO; cat duro && echo LEE_DURO; \
                      cp \"$1/kanbania.sqlite3\" copia && echo COPIA; \
                      mv \"$1/kanbania.sqlite3\" movido && echo MUEVE; \
                      /bin/launchctl list > /dev/null && echo LAUNCHCTL; \
                      /usr/bin/open -h > /dev/null 2>&1; [ $? -ne 126 ] && echo OPEN; \
                      /usr/bin/osascript -e 1 && echo OSASCRIPT; pwd -P";
        let (program, args) = c.sandboxed(
            Path::new("/bin/sh"),
            &[
                "-c".into(),
                script.into(),
                "sh".into(),
                c.data_dir().into(),
                upper.into(),
            ],
        );
        let out = std::process::Command::new(program)
            .args(args)
            .current_dir(&worktree)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(!stdout.contains("LEE_BASE"), "{stdout}");
        assert!(!stdout.contains("LISTA"), "{stdout}");
        assert!(!stdout.contains("ESCRIBE_DATOS"), "{stdout}");
        for escape in [
            "LEE_MAYUSCULAS",
            "LEE_SIMBOLICO",
            "CREA_DURO",
            "LEE_DURO",
            "COPIA",
            "MUEVE",
            "LAUNCHCTL",
            "OPEN",
            "OSASCRIPT",
        ] {
            assert!(!stdout.contains(escape), "{escape}: {stdout}");
        }
        assert!(data.join("kanbania.sqlite3").exists());
        assert!(stdout.contains("worktree"), "{stdout}");
        assert!(stdout.contains("ESCRIBE_WT"), "{stdout}");
        assert!(stdout.contains(c.worktree()), "{stdout}");
        assert!(!data.join("plantado").exists());
    }
}
