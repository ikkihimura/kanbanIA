//! Perfiles de los adaptadores ACP (HU-019: Claude Code; HU-021: Codex): lo que la app exige a
//! cada agente además del protocolo. El cliente es genérico; el perfil decide el modo
//! obligatorio, el aislamiento que se pide en `session/new` y qué opciones de permiso se
//! pueden elegir. El entorno del proceso lo prepara quien lanza el adaptador ([`codex_env`]).
//!
//! Este módulo no depende del núcleo: el supervisor traduce el agente del núcleo a un perfil.

use std::ffi::OsString;
use std::path::Path;

/// Aislamiento que se pide al agente en `session/new`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionIsolation {
    /// Nada en `session/new` (el aislamiento, si lo hay, va por el entorno del proceso).
    None,
    /// Claude Code: `_meta` de [`claude_code_isolation_meta`] para que no cargue la
    /// configuración del usuario, del proyecto ni local (sus reglas `allow`, hooks o
    /// `.mcp.json` evitarían `session/request_permission`).
    ClaudeCodeSettings,
}

/// Perfil de un adaptador ACP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcpProfile {
    /// Modo de sesión que se fija con `session/set_mode` y que el agente no puede abandonar
    /// (ni con `current_mode_update` ni con `config_option_update` de la opción de modo).
    /// Si el agente no lo ofrece → [`super::AcpFailure::ModeUnavailable`].
    pub required_mode: &'static str,
    pub isolation: SessionIsolation,
    /// Lista de permitidos de `optionId` de tipo `allow_once` que valen **solo para esa
    /// acción**. `None` = cualquier `allow_once` (Claude Code). Con `Some`, un `allow_once` que
    /// no esté en la lista no se elige nunca (Codex: `allow_permissions_turn*` concede permisos
    /// para todo el turno; `implement_plan` cambia de modo): si no queda ninguno, la persona
    /// solo puede rechazar (falla cerrado ante opciones nuevas del adaptador).
    pub allowed_allow_options: Option<&'static [&'static str]>,
    /// `optionId` de tipo `reject_once` que, además de rechazar, **terminan el turno** del
    /// agente (Codex: `cancel`). Para rechazar se prefiere cualquier otro `reject_once`; uno de
    /// estos solo como último recurso. Solo si la app envió una de estas (nunca por
    /// responder `cancelled`) y después el agente termina con `stopReason: cancelled`, el turno acaba como
    /// [`super::TurnEnd::StoppedAfterRejection`] en lugar de error de protocolo.
    pub turn_ending_reject_options: &'static [&'static str],
    /// Entradas de la raíz del worktree que contienen configuración que el agente cargaría al
    /// abrir la sesión (en cualquier forma, también un enlace roto): si hay alguna, no se lanza
    /// el adaptador (Codex: `.codex`, ver [`has_repository_config`]).
    pub repository_config: &'static [&'static str],
}

/// Claude Code (`@agentclientprotocol/claude-agent-acp`): `default` = «Always ask before making
/// changes» (comprobado con el adaptador real 0.107.0; su modo inicial es `auto`).
pub const CLAUDE_CODE_PROFILE: AcpProfile = AcpProfile {
    required_mode: "default",
    isolation: SessionIsolation::ClaudeCodeSettings,
    allowed_allow_options: None,
    turn_ending_reject_options: &[],
    repository_config: &[],
};

/// Codex (`@agentclientprotocol/codex-acp` 2.5.0, `dist/index.js`, `src/permissions/options.ts`):
/// `read-only` = aprobación «on-request» con sandbox de solo lectura y sin red: pide permiso
/// para editar, para la red y para salir del sandbox. Los comandos de solo lectura se ejecutan
/// sin preguntar y la lectura no se limita al `cwd` (limitación documentada).
///
/// Opciones de permiso de Codex: comandos → `allow_once`, `allow_for_session`,
/// `accept_execpolicy_amendment` (estas dos, `allow_always`: nunca se eligen), `decline`
/// (rechaza y el turno sigue) y `cancel` (rechaza y **aborta el turno**); ediciones →
/// `allow_once`, `allow_for_session` y solo `cancel` para rechazar; permisos adicionales →
/// `allow_permissions_turn`/`allow_permissions_turn_strict_auto_review` (`allow_once` pero
/// para todo el turno: excluidas), `allow_permissions_session` y `reject_permissions`.
pub const CODEX_PROFILE: AcpProfile = AcpProfile {
    required_mode: "read-only",
    isolation: SessionIsolation::None,
    // Las únicas de 2.5.0 que valen para una sola acción (`dist/index.js`): `allow_once`
    // (comandos, ediciones y herramientas MCP) y `accept` (peticiones MCP que no son de
    // herramienta, sin `persist`).
    allowed_allow_options: Some(&["allow_once", "accept"]),
    turn_ending_reject_options: &["cancel"],
    repository_config: &[CODEX_REPOSITORY_CONFIG],
};

/// `_meta` de `session/new` que aísla Claude Code (adaptador `claude-agent-acp`, comprobado en
/// su `dist/new-session-meta.d.ts` 0.107.0: `_meta.claudeCode.options` son opciones del SDK
/// `@anthropic-ai/claude-agent-sdk`). `settingSources: []` = no cargar ajustes de usuario,
/// proyecto ni local (la política gestionada sí se aplica);
/// `allowDangerouslySkipPermissions: false` quita `bypassPermissions`.
pub fn claude_code_isolation_meta() -> serde_json::Value {
    serde_json::json!({
        "claudeCode": {
            "options": {
                "settingSources": [],
                "allowDangerouslySkipPermissions": false
            }
        }
    })
}

/// Línea `sys` fija cuando se pide el aislamiento de Claude Code.
pub const ISOLATION_NOTE: &str =
    "La app pidió no cargar la configuración de Claude Code (usuario, proyecto ni local).";

/// Línea `sys` fija al lanzar Codex (sin rutas: la carpeta es local de la persona).
pub const CODEX_HOME_NOTE: &str = "La app usa su propia carpeta de Codex y su propia carpeta \
     personal (no carga ~/.codex ni ~/.agents): sin la configuración, los servidores MCP, las \
     reglas, el historial ni las skills del usuario, ni los plugins remotos de la cuenta. \
     Búsqueda web, apps, hooks, memorias, historial y analítica desactivados. Codex sigue \
     listando sus propias skills del sistema.";

/// Carpeta de configuración de Codex dentro de un repositorio. Codex 2.5.0 la carga al abrir
/// la sesión (el adaptador marca el `cwd` como confiable): servidores MCP que se lanzan antes
/// del primer prompt, reglas y ajustes que evitarían los permisos.
pub const CODEX_REPOSITORY_CONFIG: &str = ".codex";

/// `CODEX_CONFIG` que acepta `codex-acp` 2.5.0 (comprobado sin prompt): sin búsqueda web, apps,
/// hooks, memorias, plugins remotos de la cuenta (`remote_plugin`: sus skills dejan de
/// listarse), historial persistente, analítica ni skills incluidas. Comprobado por el
/// propietario: `skills/list` sigue mostrando las skills del sistema de Codex pese a
/// `skills.bundled.enabled = false`.
pub const CODEX_CONFIG: &str = r#"{"web_search":"disabled","features":{"apps":false,"hooks":false,"memories":false,"remote_plugin":false},"history":{"persistence":"none"},"analytics":{"enabled":false},"skills":{"bundled":{"enabled":false}}}"#;

/// Carpeta personal (`HOME`) que la app da a Codex, dentro de su carpeta propia: Codex busca
/// skills del usuario en `$HOME/.agents/skills` aunque `CODEX_HOME` sea otro; con una carpeta
/// vacía no ve las del usuario (comprobado sin prompt: la sesión sigue funcionando y no escribe
/// nada en ella).
pub const CODEX_PERSONAL_HOME: &str = "home";

/// Variables que la app fija para Codex, además de la lista de adaptadores: carpeta propia
/// (`CODEX_HOME`), carpeta personal propia (`HOME`, y en Windows `USERPROFILE`, que sustituyen
/// a las del usuario de la lista común: [`CODEX_PERSONAL_HOME`]), modo inicial `read-only` (antes de `session/set_mode`), sin abrir el
/// navegador y [`CODEX_CONFIG`]. Nunca `CODEX_API_KEY`, `CODEX_PATH`, `OPENAI_*`,
/// `MODEL_PROVIDER`, `DEFAULT_AUTH_REQUEST` ni `APP_SERVER_LOGS` (ver
/// [`crate::is_adapter_forbidden`]): la sesión iniciada en la carpeta propia basta.
pub fn codex_env(codex_home: &Path) -> Vec<(OsString, OsString)> {
    let personal = codex_home.join(CODEX_PERSONAL_HOME).into_os_string();
    vec![
        ("CODEX_HOME".into(), codex_home.as_os_str().to_owned()),
        ("HOME".into(), personal.clone()),
        #[cfg(windows)]
        ("USERPROFILE".into(), personal),
        (
            "INITIAL_AGENT_MODE".into(),
            CODEX_PROFILE.required_mode.into(),
        ),
        ("NO_BROWSER".into(), "1".into()),
        ("CODEX_CONFIG".into(), CODEX_CONFIG.into()),
    ]
}

/// Crea la carpeta de Codex de la app y su carpeta personal ([`CODEX_PERSONAL_HOME`]) si faltan
/// (0700 en Unix) y, si existen, comprueba que son carpetas reales (no enlaces) y las deja en
/// 0700. E/S bloqueante y breve.
pub fn ensure_codex_home(path: &Path) -> std::io::Result<()> {
    ensure_private_dir(path)?;
    ensure_private_dir(&path.join(CODEX_PERSONAL_HOME))
}

fn ensure_private_dir(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "la carpeta de Codex no es una carpeta",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg_attr(not(unix), allow(unused_mut))]
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(path)?;
        }
        Err(error) => return Err(error),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Primera entrada de [`AcpProfile::repository_config`] presente en la raíz del worktree (en
/// cualquier forma: carpeta, archivo o enlace, aunque esté roto). `Err` si no se puede saber
/// (quien llama falla cerrado). E/S bloqueante y breve.
pub fn has_repository_config(
    profile: &AcpProfile,
    worktree: &Path,
) -> std::io::Result<Option<&'static str>> {
    for name in profile.repository_config {
        if exists_in_any_form(&worktree.join(name))? {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// Entradas de la raíz de la carpeta de Codex de la app que Codex carga como configuración,
/// reglas, instrucciones, prompts, hooks o notificaciones (podrían saltarse los permisos o
/// ejecutar programas). La app nunca las crea; la persona solo debe iniciar sesión.
pub const CODEX_HOME_DENYLIST: &[&str] = &[
    "config.toml",
    "config.json",
    "managed_config.toml",
    "requirements.toml",
    "rules",
    "AGENTS.md",
    "AGENTS.override.md",
    "prompts",
    "hooks",
    "hooks.json",
    "notify",
];

/// Cachés de plugins que Codex 2.5.0 descarga en su carpeta **en cada sesión**, aunque
/// `features.remote_plugin = false` (comprobado: tanto en `CODEX_CONFIG` como en un
/// `config.toml`): los plugins remotos de la cuenta de ChatGPT y su catálogo. La app los borra
/// antes de cada lanzamiento ([`clear_codex_plugin_cache`]): así nunca sobrevive de una sesión
/// a otra un plugin que alguien haya puesto ahí.
pub const CODEX_PLUGIN_CACHES: &[&str] = &["plugins", "cache/remote_plugin_catalog"];

/// Borra las [`CODEX_PLUGIN_CACHES`] de la carpeta de Codex de la app sin seguir enlaces (un
/// enlace se borra como enlace, nunca su destino). E/S bloqueante y breve.
pub fn clear_codex_plugin_cache(home: &Path) -> std::io::Result<()> {
    for name in CODEX_PLUGIN_CACHES {
        let path = home.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(&path)?,
            Ok(_) => std::fs::remove_file(&path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Subcarpeta de skills de Codex y la única entrada que Codex crea en ella por su cuenta
/// (skills del sistema, observado con 2.5.0 en una carpeta nueva sin ningún prompt).
const CODEX_SKILLS_DIR: &str = "skills";
const CODEX_SYSTEM_SKILLS: &str = ".system";

/// Entradas de la carpeta personal de Codex ([`CODEX_PERSONAL_HOME`]) que no se admiten:
/// skills del usuario (`.agents`) y una `~/.codex` (un agente con una escritura permitida
/// podría plantarlas para la siguiente ejecución).
pub const CODEX_PERSONAL_DENYLIST: &[&str] = &["home/.agents", "home/.codex"];

/// Comprueba la carpeta de Codex de la app antes de cada lanzamiento. Devuelve la entrada que
/// lo impide (un nombre fijo de la lista, sin rutas) o `None`:
/// - cualquier entrada de [`CODEX_HOME_DENYLIST`] o de [`CODEX_PERSONAL_DENYLIST`] en
///   cualquier forma;
/// - `skills` que no sea una carpeta real, o que contenga algo distinto de `.system` (carpeta
///   real): las skills del sistema las crea Codex solo (decisión: se admiten; modificarlas exige
///   escribir en el directorio de datos, fuera del modelo de amenaza), las de la persona no.
///
/// `Err` si no se puede leer (quien llama falla cerrado). E/S bloqueante y breve.
pub fn codex_home_config(home: &Path) -> std::io::Result<Option<&'static str>> {
    for name in CODEX_HOME_DENYLIST.iter().chain(CODEX_PERSONAL_DENYLIST) {
        if exists_in_any_form(&home.join(name))? {
            return Ok(Some(name));
        }
    }
    let skills = home.join(CODEX_SKILLS_DIR);
    match std::fs::symlink_metadata(&skills) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
        Ok(metadata) if !metadata.is_dir() => return Ok(Some(CODEX_SKILLS_DIR)),
        Ok(_) => {}
    }
    for entry in std::fs::read_dir(&skills)? {
        let entry = entry?;
        let system = entry.file_name() == CODEX_SYSTEM_SKILLS
            && std::fs::symlink_metadata(entry.path())?.is_dir();
        if !system {
            return Ok(Some(CODEX_SKILLS_DIR));
        }
    }
    Ok(None)
}

fn exists_in_any_form(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_env_is_minimal_and_allowed() {
        let env = codex_env(Path::new("/datos/codex-home"));
        #[cfg_attr(not(unix), allow(unused_variables))]
        let names: Vec<_> = env.iter().map(|(n, _)| n.to_str().unwrap()).collect();
        #[cfg(unix)]
        assert_eq!(
            names,
            [
                "CODEX_HOME",
                "HOME",
                "INITIAL_AGENT_MODE",
                "NO_BROWSER",
                "CODEX_CONFIG"
            ]
        );
        let personal = Path::new("/datos/codex-home").join("home").into_os_string();
        assert!(env.contains(&("HOME".into(), personal)));
        for (name, _) in &env {
            assert!(!crate::is_adapter_forbidden(name), "{name:?}");
        }
        let config: serde_json::Value = serde_json::from_str(CODEX_CONFIG).unwrap();
        assert_eq!(config["web_search"], "disabled");
        assert_eq!(config["history"]["persistence"], "none");
        assert_eq!(config["features"]["remote_plugin"], false);
    }

    #[test]
    fn codex_home_is_private_and_never_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("codex-home");
        assert!(ensure_codex_home(&home).is_ok());
        assert!(ensure_codex_home(&home).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&home).map(|m| m.permissions().mode() & 0o777);
            assert_eq!(mode.ok(), Some(0o700));
            let link = dir.path().join("enlace");
            assert!(std::os::unix::fs::symlink(&home, &link).is_ok());
            assert!(ensure_codex_home(&link).is_err());
        }
        assert_eq!(
            has_repository_config(&CODEX_PROFILE, dir.path()).unwrap(),
            None
        );
        std::fs::write(dir.path().join(".codex"), "").unwrap();
        assert_eq!(
            has_repository_config(&CODEX_PROFILE, dir.path()).unwrap(),
            Some(".codex")
        );
        assert_eq!(
            has_repository_config(&CLAUDE_CODE_PROFILE, dir.path()).unwrap(),
            None
        );
    }

    #[test]
    fn codex_home_only_admits_system_skills() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        assert_eq!(codex_home_config(home).unwrap(), None);
        std::fs::create_dir_all(home.join("skills").join(".system").join("x")).unwrap();
        std::fs::write(home.join("auth.json"), "{}").unwrap();
        assert_eq!(codex_home_config(home).unwrap(), None);
        std::fs::create_dir(home.join("skills").join("mia")).unwrap();
        assert_eq!(codex_home_config(home).unwrap(), Some("skills"));
        std::fs::remove_dir(home.join("skills").join("mia")).unwrap();
        std::fs::create_dir_all(home.join("home").join(".agents")).unwrap();
        assert_eq!(codex_home_config(home).unwrap(), Some("home/.agents"));
        std::fs::remove_dir(home.join("home").join(".agents")).unwrap();
        std::fs::write(home.join("home").join(".codex"), "").unwrap();
        assert_eq!(codex_home_config(home).unwrap(), Some("home/.codex"));
        std::fs::remove_file(home.join("home").join(".codex")).unwrap();
        assert_eq!(codex_home_config(home).unwrap(), None);
        for name in CODEX_HOME_DENYLIST {
            std::fs::write(home.join(name), "").unwrap();
            assert_eq!(codex_home_config(home).unwrap(), Some(*name));
            std::fs::remove_file(home.join(name)).unwrap();
        }
    }
}
