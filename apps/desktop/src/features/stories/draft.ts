import type { Priority } from "../../contracts/generated/Priority";
import type { StoryDetail } from "../../contracts/generated/StoryDetail";
import type { UpdateStoryInput } from "../../contracts/generated/UpdateStoryInput";

export interface CriterionRow {
  /** Clave estable para React; para criterios guardados coincide con `id`. */
  key: string;
  id?: string;
  /** Número CA-n si ya existe. */
  number?: number;
  text: string;
}

export interface StoryDraft {
  title: string;
  description: string;
  priority: Priority;
  criteria: CriterionRow[];
}

export function draftFrom(story: StoryDetail): StoryDraft {
  return {
    title: story.title,
    description: story.description,
    priority: story.priority,
    criteria: story.criteria.map((c) => ({
      key: c.id,
      id: c.id,
      number: c.number,
      text: c.text,
    })),
  };
}

export function sameDraft(a: StoryDraft, b: StoryDraft): boolean {
  return (
    a.title === b.title &&
    a.description === b.description &&
    a.priority === b.priority &&
    a.criteria.length === b.criteria.length &&
    a.criteria.every(
      (c, i) => c.id === b.criteria[i]?.id && c.text === b.criteria[i]?.text,
    )
  );
}

/**
 * Entrada de guardado. Frente a la versión `base`, los criterios cuyo id ya no existe en ella
 * (otro editor los borró) se reenvían como nuevos: así "conservar mi versión" no los pierde.
 */
export function toUpdateInput(
  base: StoryDetail,
  draft: StoryDraft,
): UpdateStoryInput {
  const known = new Set(base.criteria.map((c) => c.id));
  return {
    projectId: base.projectId,
    id: base.id,
    expectedRevision: base.revision,
    title: draft.title,
    description: draft.description,
    priority: draft.priority,
    criteria: draft.criteria.map((c) =>
      c.id && known.has(c.id) ? { id: c.id, text: c.text } : { text: c.text },
    ),
  };
}
