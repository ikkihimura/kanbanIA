//! Salida de un proceso en JSONL acotado y rotado (nunca en SQLite ni acumulada en memoria).
//!
//! Cada línea es un objeto `{"t":ms,"s":"out"|"err"|"sys"|"agent","l":"texto"}` (+ `"cut":true`
//! si se cortó). `sys` son mensajes del propio runner (p. ej. el detalle de un error de Git),
//! nunca del proceso. `agent` (HU-019) es lo que un agente **reporta** vía ACP, ya normalizado
//! por el runner, con su tipo en `"k"` (`message`, `thought`, `tool`, `plan`, `usage`,
//! `permission`); `out`/`err` son lo observado en stdout/stderr. Archivos `output-<n>.jsonl` con `n` creciente desde 1: se rota al superar
//! [`MAX_FILE_BYTES`] y se conservan como mucho [`MAX_FILES`] (los más viejos se borran, y lo
//! borrado se cuenta en [`OutputStats`]).
//!
//! El texto del proceso es dato no confiable: se guarda tal cual (como texto JSON escapado) y
//! la UI debe mostrarlo como texto plano, nunca como Markdown ni HTML.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Bytes máximos de texto por línea; el resto se descarta y la línea se marca `cut`.
pub const MAX_LINE_BYTES: usize = 4 * 1024;
/// Tamaño a partir del cual se abre un archivo nuevo.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Archivos que se conservan como mucho (≈ 4 MiB de salida por intento).
pub const MAX_FILES: usize = 4;
/// Líneas máximas que devuelve [`read_tail`].
pub const MAX_TAIL_LINES: usize = 500;

const FILE_PREFIX: &str = "output-";
const FILE_SUFFIX: &str = ".jsonl";
/// Lo que se lee como mucho de un archivo al buscar la cola (holgura sobre `MAX_FILE_BYTES`:
/// un archivo puede pasarse en un registro).
const MAX_TAIL_READ: u64 = 2 * MAX_FILE_BYTES;

/// Origen de una línea.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stream {
    #[serde(rename = "out")]
    Stdout,
    #[serde(rename = "err")]
    Stderr,
    /// Mensaje del runner (no del proceso).
    #[serde(rename = "sys")]
    System,
    /// Reportado por el agente vía ACP y normalizado por el runner (HU-019).
    #[serde(rename = "agent")]
    Agent,
}

/// Tipo de una línea `agent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    /// Texto de un mensaje del agente.
    Message,
    /// «Pensando…» con la longitud, sin el contenido.
    Thought,
    /// Herramienta: título, tipo y estado (sin entrada ni contenido).
    Tool,
    /// Plan: entradas con su estado.
    Plan,
    /// Uso de contexto y coste tal como los informa el agente.
    Usage,
    /// Permiso pedido y la respuesta de la app.
    Permission,
}

/// Una línea guardada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputLine {
    /// Hora de recepción, ms Unix (reloj del sistema: es informativa, ninguna regla depende de
    /// ella).
    #[serde(rename = "t")]
    pub time_ms: i64,
    #[serde(rename = "s")]
    pub stream: Stream,
    #[serde(rename = "l")]
    pub text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut: bool,
    /// Tipo de una línea `agent` (en las demás, ausente).
    #[serde(rename = "k", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<AgentKind>,
}

/// Recuento de lo escrito y lo descartado.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct OutputStats {
    /// Líneas escritas.
    pub lines: u64,
    /// Líneas cortadas a [`MAX_LINE_BYTES`].
    pub cut_lines: u64,
    /// Bytes descartados al cortar líneas (en texto no UTF-8, sobre el texto ya convertido).
    pub cut_bytes: u64,
    /// Archivos viejos borrados por la rotación.
    pub dropped_files: u64,
    /// Bytes de esos archivos.
    pub dropped_bytes: u64,
    /// Líneas perdidas por errores de escritura (disco lleno…). El proceso no se detiene.
    pub lost_lines: u64,
}

/// Escritor del JSONL de un intento. Síncrono: el proceso lo usa desde un hilo bloqueante.
#[derive(Debug)]
pub struct OutputLog {
    dir: PathBuf,
    /// Archivos en disco, del más viejo al más nuevo: (índice, tamaño).
    files: VecDeque<(u64, u64)>,
    current: Option<BufWriter<File>>,
    stats: OutputStats,
}

impl OutputLog {
    /// Abre el registro en `dir` (lo crea si falta). Si ya hay archivos, continúa con el índice
    /// siguiente: nunca sobrescribe.
    pub fn create(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let mut existing = list_files(dir)?;
        existing.sort_unstable();
        let files = existing
            .into_iter()
            .map(|(index, path)| {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                (index, size)
            })
            .collect();
        let mut log = Self {
            dir: dir.to_path_buf(),
            files,
            current: None,
            stats: OutputStats::default(),
        };
        log.open_next()?;
        Ok(log)
    }

    pub fn stats(&self) -> OutputStats {
        self.stats
    }

    /// Escribe una línea (sin el salto final). `already_cut` = bytes que quien lee ya descartó
    /// por exceder [`MAX_LINE_BYTES`].
    pub fn write_line(&mut self, stream: Stream, bytes: &[u8], already_cut: u64) {
        self.write_record(stream, None, bytes, already_cut);
    }

    /// Línea `agent` con su tipo. El texto es dato no confiable del agente.
    pub fn write_agent(&mut self, kind: AgentKind, text: &str) {
        self.write_record(Stream::Agent, Some(kind), text.as_bytes(), 0);
    }

    /// Escribe un registro (sin el salto final). Una línea `agent` sin tipo no se escribe.
    pub(crate) fn write_record(
        &mut self,
        stream: Stream,
        kind: Option<AgentKind>,
        bytes: &[u8],
        already_cut: u64,
    ) {
        let kind = if stream == Stream::Agent { kind } else { None };
        if stream == Stream::Agent && kind.is_none() {
            return;
        }
        let text = String::from_utf8_lossy(bytes);
        let (text, extra_cut) = truncate_at_char(&text, MAX_LINE_BYTES);
        let cut_bytes = already_cut + extra_cut as u64;
        let line = OutputLine {
            time_ms: now_ms(),
            stream,
            text: text.to_owned(),
            cut: cut_bytes > 0,
            kind,
        };
        if cut_bytes > 0 {
            self.stats.cut_lines += 1;
            self.stats.cut_bytes += cut_bytes;
        }
        if self.append(&line).is_ok() {
            self.stats.lines += 1;
        } else {
            self.stats.lost_lines += 1;
        }
    }

    /// Mensaje del runner (detalle de un error, motivo de cierre…).
    pub fn write_system(&mut self, text: &str) {
        for line in text.lines() {
            self.write_line(Stream::System, line.as_bytes(), 0);
        }
    }

    /// Vuelca lo pendiente al disco (para que la UI lo pueda leer).
    pub fn flush(&mut self) -> io::Result<()> {
        match self.current.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }

    /// Vuelca y cierra; devuelve el recuento final.
    pub fn finish(mut self) -> OutputStats {
        let _ = self.flush();
        self.stats
    }

    fn append(&mut self, line: &OutputLine) -> io::Result<()> {
        let mut record = serde_json::to_vec(line).map_err(io::Error::other)?;
        record.push(b'\n');
        let len = record.len() as u64;
        let current_size = self.files.back().map_or(0, |(_, size)| *size);
        if self.current.is_none() || (current_size > 0 && current_size + len > MAX_FILE_BYTES) {
            self.open_next()?;
        }
        let file = self
            .current
            .as_mut()
            .ok_or_else(|| io::Error::other("sin archivo de salida"))?;
        file.write_all(&record)?;
        if let Some((_, size)) = self.files.back_mut() {
            *size += len;
        }
        Ok(())
    }

    /// Cierra el archivo actual, abre el siguiente y borra los que sobran.
    fn open_next(&mut self) -> io::Result<()> {
        if let Some(mut file) = self.current.take() {
            let _ = file.flush();
        }
        let index = self.files.back().map_or(1, |(index, _)| index + 1);
        let file = create_new(&file_path(&self.dir, index))?;
        self.current = Some(BufWriter::new(file));
        self.files.push_back((index, 0));
        while self.files.len() > MAX_FILES {
            if let Some((old, size)) = self.files.pop_front()
                && std::fs::remove_file(file_path(&self.dir, old)).is_ok()
            {
                self.stats.dropped_files += 1;
                self.stats.dropped_bytes += size;
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn create_new(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    // Solo el usuario: la salida puede contener datos sensibles del repositorio.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_new(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Últimas líneas de un intento, de la más vieja a la más nueva.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct OutputTail {
    pub lines: Vec<OutputLine>,
    /// Líneas ilegibles que se saltaron (p. ej. la última, a medio escribir).
    pub skipped_corrupt: u64,
    /// `true` si la rotación ya borró salida anterior a la que se conserva.
    pub earlier_dropped: bool,
}

/// Lee las últimas `max_lines` líneas (como mucho [`MAX_TAIL_LINES`]) guardadas en `dir`.
/// Una carpeta que no existe es una salida vacía. Las líneas corruptas se saltan.
pub fn read_tail(dir: &Path, max_lines: usize) -> io::Result<OutputTail> {
    let max_lines = max_lines.min(MAX_TAIL_LINES);
    let mut files = match list_files(dir) {
        Ok(files) => files,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(OutputTail::default()),
        Err(error) => return Err(error),
    };
    files.sort_unstable_by_key(|file| std::cmp::Reverse(file.0));
    let mut tail = OutputTail {
        earlier_dropped: files.last().is_some_and(|(index, _)| *index > 1),
        ..OutputTail::default()
    };
    // Recorre del archivo más nuevo al más viejo; dentro de cada uno, de la última línea a la
    // primera.
    let mut reversed: Vec<OutputLine> = Vec::new();
    for (_, path) in files {
        if reversed.len() >= max_lines {
            break;
        }
        let Some(raw_lines) = read_file_lines(&path)? else {
            continue;
        };
        for raw in raw_lines.iter().rev() {
            if reversed.len() >= max_lines {
                break;
            }
            match serde_json::from_slice::<OutputLine>(raw) {
                Ok(line) => reversed.push(line),
                Err(_) => tail.skipped_corrupt += 1,
            }
        }
    }
    reversed.reverse();
    tail.lines = reversed;
    Ok(tail)
}

/// Líneas en bruto de un archivo normal (sin seguir enlaces); `None` si no es un archivo
/// normal. Si es mayor que [`MAX_TAIL_READ`], solo se lee el final (sin la primera línea,
/// incompleta).
fn read_file_lines(path: &Path) -> io::Result<Option<Vec<Vec<u8>>>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() {
        return Ok(None);
    }
    let mut file = match File::open(path) {
        Ok(file) => file,
        // Borrado por la rotación entre listar y abrir.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut skip_first = false;
    if metadata.len() > MAX_TAIL_READ {
        file.seek(SeekFrom::Start(metadata.len() - MAX_TAIL_READ))?;
        skip_first = true;
    }
    let mut reader = BufReader::new(file.take(MAX_TAIL_READ));
    let mut lines = Vec::new();
    loop {
        let mut line = Vec::new();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if skip_first {
            skip_first = false;
            continue;
        }
        while line.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
            line.pop();
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    Ok(Some(lines))
}

/// Archivos `output-<n>.jsonl` de la carpeta, con su índice.
fn list_files(dir: &Path) -> io::Result<Vec<(u64, PathBuf)>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(index) = name
            .strip_prefix(FILE_PREFIX)
            .and_then(|rest| rest.strip_suffix(FILE_SUFFIX))
            .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|digits| digits.parse::<u64>().ok())
        else {
            continue;
        };
        if index > 0 {
            files.push((index, entry.path()));
        }
    }
    Ok(files)
}

fn file_path(dir: &Path, index: u64) -> PathBuf {
    dir.join(format!("{FILE_PREFIX}{index}{FILE_SUFFIX}"))
}

/// Recorta `text` a `max` bytes sin partir un carácter; devuelve los bytes quitados.
pub(crate) fn truncate_at_char(text: &str, max: usize) -> (&str, usize) {
    if text.len() <= max {
        return (text, 0);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], text.len() - end)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_on_char_boundary() {
        let text = "aé"; // 'é' ocupa 2 bytes
        assert_eq!(truncate_at_char(text, 2), ("a", 2));
        assert_eq!(truncate_at_char(text, 3), ("aé", 0));
    }

    #[test]
    fn rotates_and_keeps_at_most_four_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = OutputLog::create(dir.path()).unwrap();
        let line = vec![b'x'; 1000];
        for _ in 0..6000 {
            log.write_line(Stream::Stdout, &line, 0);
        }
        let stats = log.finish();
        let files = list_files(dir.path()).unwrap();
        assert_eq!(files.len(), MAX_FILES);
        assert!(stats.dropped_files >= 1);
        for (_, path) in &files {
            assert!(std::fs::metadata(path).unwrap().len() <= MAX_FILE_BYTES);
        }
        let tail = read_tail(dir.path(), 10_000).unwrap();
        assert_eq!(tail.lines.len(), MAX_TAIL_LINES);
        assert!(tail.earlier_dropped);
    }

    #[test]
    fn tail_skips_corrupt_lines_and_missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("output-1.jsonl"),
            "{\"t\":1,\"s\":\"out\",\"l\":\"uno\"}\nbasura\n{\"t\":2,\"s\":\"err\",\"l\":\"dos\",\"cut\":true}\n{\"t\":3,",
        )
        .unwrap();
        let tail = read_tail(dir.path(), 10).unwrap();
        assert_eq!(tail.skipped_corrupt, 2);
        assert_eq!(tail.lines.len(), 2);
        assert_eq!(tail.lines[0].text, "uno");
        assert!(tail.lines[1].cut);
        assert_eq!(tail.lines[1].stream, Stream::Stderr);
        assert!(!tail.earlier_dropped);

        let missing = read_tail(&dir.path().join("no-existe"), 10).unwrap();
        assert!(missing.lines.is_empty());
    }

    #[test]
    fn agent_lines_carry_their_kind() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = OutputLog::create(dir.path()).unwrap();
        log.write_agent(AgentKind::Tool, "Leer README.md");
        log.write_line(Stream::Stderr, b"diagnostico", 0);
        // Una línea `agent` sin tipo no se escribe; un tipo en otro origen se descarta.
        log.write_record(Stream::Agent, None, b"sin tipo", 0);
        log.write_record(Stream::System, Some(AgentKind::Message), b"sys", 0);
        log.finish();
        let raw = std::fs::read_to_string(dir.path().join("output-1.jsonl")).unwrap();
        assert!(
            raw.contains("\"s\":\"agent\",\"l\":\"Leer README.md\",\"k\":\"tool\""),
            "{raw}"
        );
        let tail = read_tail(dir.path(), 10).unwrap();
        assert_eq!(tail.lines.len(), 3);
        assert_eq!(tail.lines[0].kind, Some(AgentKind::Tool));
        assert_eq!(tail.lines[1].kind, None);
        assert_eq!(tail.lines[2].stream, Stream::System);
        assert_eq!(tail.lines[2].kind, None);
    }

    #[test]
    fn reopening_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = OutputLog::create(dir.path()).unwrap();
        log.write_system("primero");
        log.finish();
        let mut log = OutputLog::create(dir.path()).unwrap();
        log.write_system("segundo");
        log.finish();
        let tail = read_tail(dir.path(), 10).unwrap();
        let texts: Vec<_> = tail.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["primero", "segundo"]);
    }
}
