//! Transporte stdio de ACP v1: JSON-RPC 2.0 delimitado por saltos de línea (un mensaje por
//! línea, sin saltos dentro). Una línea entrante de más de [`MAX_LINE_BYTES`] es un error de
//! protocolo: nunca se acumula sin límite.

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// Bytes máximos de una línea entrante (sin el salto).
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Resultado de leer una línea.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReadLine {
    Line(Vec<u8>),
    /// Fin de la salida (o una línea a medias al final, que no se puede interpretar).
    Eof,
    /// La línea supera [`MAX_LINE_BYTES`].
    TooLong,
}

/// Lector de líneas acotadas. Seguro ante cancelación (`select!`): lo leído a medias se
/// conserva para la siguiente llamada.
pub(crate) struct LineReader<R> {
    inner: BufReader<R>,
    line: Vec<u8>,
}

impl<R: AsyncRead + Unpin> LineReader<R> {
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner: BufReader::with_capacity(64 * 1024, inner),
            line: Vec::new(),
        }
    }

    pub(crate) async fn next_line(&mut self) -> ReadLine {
        loop {
            let available = match self.inner.fill_buf().await {
                Ok(available) => available,
                Err(_) => return ReadLine::Eof,
            };
            if available.is_empty() {
                self.line.clear();
                return ReadLine::Eof;
            }
            match available.iter().position(|b| *b == b'\n') {
                Some(at) => {
                    if self.line.len() + at > MAX_LINE_BYTES {
                        return ReadLine::TooLong;
                    }
                    self.line.extend_from_slice(&available[..at]);
                    self.inner.consume(at + 1);
                    let mut line = std::mem::take(&mut self.line);
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    return ReadLine::Line(line);
                }
                None => {
                    let len = available.len();
                    if self.line.len() + len > MAX_LINE_BYTES {
                        return ReadLine::TooLong;
                    }
                    self.line.extend_from_slice(available);
                    self.inner.consume(len);
                }
            }
        }
    }
}

/// Escribe un mensaje (una línea) y vacía el búfer. `serde_json` nunca emite saltos de línea
/// sin escapar en compacto.
pub(crate) async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: &serde_json::Value,
) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(message).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn splits_lines_and_rejects_oversize() {
        let mut data = b"{\"a\":1}\r\n\n{\"b\":2}\n".to_vec();
        data.extend(std::iter::repeat_n(b'x', MAX_LINE_BYTES + 1));
        data.push(b'\n');
        let mut reader = LineReader::new(&data[..]);
        assert_eq!(
            reader.next_line().await,
            ReadLine::Line(b"{\"a\":1}".to_vec())
        );
        assert_eq!(reader.next_line().await, ReadLine::Line(Vec::new()));
        assert_eq!(
            reader.next_line().await,
            ReadLine::Line(b"{\"b\":2}".to_vec())
        );
        assert_eq!(reader.next_line().await, ReadLine::TooLong);
    }

    #[tokio::test]
    async fn partial_line_at_end_is_eof() {
        let mut reader = LineReader::new(&b"{\"a\":1}\n{\"incompleta\""[..]);
        assert!(matches!(reader.next_line().await, ReadLine::Line(_)));
        assert_eq!(reader.next_line().await, ReadLine::Eof);
    }

    #[tokio::test]
    async fn exactly_the_limit_is_accepted() {
        let mut data = vec![b'y'; MAX_LINE_BYTES];
        data.push(b'\n');
        let mut reader = LineReader::new(&data[..]);
        assert!(matches!(reader.next_line().await, ReadLine::Line(l) if l.len() == MAX_LINE_BYTES));
    }
}
