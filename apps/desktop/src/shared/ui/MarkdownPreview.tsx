import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

/**
 * Vista previa segura de Markdown escrito por el usuario o por agentes (dato no confiable):
 * - `react-markdown` crea elementos React; nunca usa `innerHTML`. `skipHtml` descarta el HTML
 *   crudo (`<script>`, `<img onerror>`…) en lugar de mostrarlo o ejecutarlo.
 * - Los enlaces no navegan (sacarían la ventana de la app); se muestran con su destino.
 * - Las imágenes no se cargan (privacidad: una URL remota revelaría actividad); se muestra su texto.
 */
const components: Components = {
  a: ({ children, href }) => (
    <span className="md-link" title={href ?? undefined}>
      {children}
      {href ? <span className="md-link__href"> ({href})</span> : null}
    </span>
  ),
  img: ({ alt }) => (
    <span className="md-image">[imagen{alt ? `: ${alt}` : ""}]</span>
  ),
};

interface Props {
  source: string;
}

export function MarkdownPreview({ source }: Props) {
  if (source.trim() === "") {
    return <p className="panel__muted">Sin descripción.</p>;
  }
  return (
    <div className="markdown">
      <Markdown remarkPlugins={[remarkGfm]} skipHtml components={components}>
        {source}
      </Markdown>
    </div>
  );
}
