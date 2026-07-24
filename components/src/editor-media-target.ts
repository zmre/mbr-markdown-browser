/**
 * Pure helpers for locating and describing a media embed that the media picker
 * can edit in place.
 *
 * Deliberately free of any Milkdown/ProseMirror *runtime* import (only an erased
 * `import type`), so this logic stays cheap and unit-testable. The plugin that
 * uses these at runtime lives in editor-media-edit.ts.
 */

import type { Node as ProseNode } from '@milkdown/kit/prose/model';

/** A media embed the media picker can edit in place. */
export interface EditTarget {
  src: string;
  caption: string;
  /** Document position directly before the node. */
  pos: number;
  nodeSize: number;
  /** `image` (inline) or `image-block` (block) — determines attr mapping. */
  typeName: 'image' | 'image-block';
}

/** The identifying fields of an {@link EditTarget}, without its position. */
export type EditFields = Pick<EditTarget, 'src' | 'caption' | 'typeName'>;

/**
 * Maps an image node's type name + attrs to the picker-facing src/caption, or
 * `null` for any non-image node. mbr embeds are authored as `![caption](url)`:
 * block images (`image-block`) keep the caption in the `caption` attr; inline
 * images (`image`) keep it in `alt`.
 */
export function imageEditFields(
  typeName: string,
  attrs: Record<string, unknown>,
): EditFields | null {
  if (typeName === 'image-block') {
    return {
      src: (attrs.src as string) ?? '',
      caption: (attrs.caption as string) ?? '',
      typeName: 'image-block',
    };
  }
  if (typeName === 'image') {
    return {
      src: (attrs.src as string) ?? '',
      caption: (attrs.alt as string) ?? '',
      typeName: 'image',
    };
  }
  return null;
}

/**
 * Builds an {@link EditTarget} for `node` located at `pos` (the position
 * directly before it), or `null` when `node` is not an image embed. `pos` and
 * `nodeSize` bound the node as `[pos, pos + nodeSize)`, matching how the caller
 * replaces it.
 */
export function editTargetFromNode(node: ProseNode, pos: number): EditTarget | null {
  const fields = imageEditFields(node.type.name, node.attrs);
  if (!fields) return null;
  return { ...fields, pos, nodeSize: node.nodeSize };
}
