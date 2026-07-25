/**
 * Double-click-to-edit for media embeds in the Crepe editor.
 *
 * Crepe renders block images (`image-block`) as an interactive *atom* NodeView
 * whose `stopEvent` only swallows events aimed at its inner `<input>`s, so
 * ordinary interaction almost never yields a ProseMirror `NodeSelection` — which
 * makes the footer media button's edit mode hard to reach. This module makes
 * editing an existing embed both discoverable and reliable via a Milkdown plugin
 * (a ProseMirror `Plugin` wrapped with `$prose`), registered on the Crepe
 * instance with `crepe.editor.use(...)` before `.create()` — the same mechanism
 * editor-link-autocomplete.ts uses. It:
 *
 *   - opens the media picker in edit mode when an `image`/`image-block` node is
 *     double-clicked (`props.handleDoubleClickOn`), and
 *   - reports, on each selection change (`view().update`), whether an embed is
 *     the current target so the footer button can relabel itself "Edit media".
 *
 * The picker/replace flow itself lives in editor-crepe.ts's `openEditor`
 * closure; this module reaches it through the injected {@link MediaEditServices}.
 */

import { $prose } from '@milkdown/kit/utils';
import { NodeSelection, Plugin, PluginKey } from '@milkdown/kit/prose/state';
import type { Selection } from '@milkdown/kit/prose/state';
import type { MilkdownPlugin } from '@milkdown/kit/ctx';
import { type EditTarget, editTargetFromNode } from './editor-media-target.js';

export interface MediaEditServices {
  /** Opens the media picker in edit mode for a double-clicked embed. */
  onEditMedia: (target: EditTarget) => void;
  /**
   * Notified whenever the cursor-adjacent embed target changes (or clears), so
   * the footer button can flip between "Edit media" and "Insert media".
   */
  onTargetChange: (target: EditTarget | null) => void;
}

const mediaEditKey = new PluginKey('mbr-media-edit');

/**
 * The embed at/adjacent to `sel` that the picker can edit: a strict
 * `NodeSelection` on an image, or an image sitting just after/before the cursor.
 * The adjacency checks cover a gap cursor next to a block image and an inline
 * image next to the text caret — cases Crepe's atom `image-block` produces far
 * more often than a `NodeSelection`. Returns `null` when the selection is not on
 * an embed.
 */
export function detectEditTarget(sel: Selection): EditTarget | null {
  if (sel instanceof NodeSelection) {
    const target = editTargetFromNode(sel.node, sel.from);
    if (target) return target;
  }
  const $from = sel.$from;
  // `nodeAfter` starts at `$from.pos`; `nodeBefore` ends there, so it starts a
  // `nodeSize` earlier.
  const after = $from.nodeAfter;
  if (after) {
    const target = editTargetFromNode(after, $from.pos);
    if (target) return target;
  }
  const before = $from.nodeBefore;
  if (before) {
    const target = editTargetFromNode(before, $from.pos - before.nodeSize);
    if (target) return target;
  }
  return null;
}

/** Stable identity for a target, so the label only updates when it changes. */
function targetKey(target: EditTarget | null): string {
  return target ? `${target.typeName}@${target.pos}` : '';
}

/**
 * Creates the double-click-to-edit plugin. Register it with
 * `crepe.editor.use(createMediaEditPlugin(services))` before `create()`.
 */
export function createMediaEditPlugin(services: MediaEditServices): MilkdownPlugin {
  return $prose(() => {
    let lastKey: string | null = null;
    return new Plugin({
      key: mediaEditKey,
      props: {
        // Double-clicking an image opens the picker in edit mode for that exact
        // node; `nodePos` is the position directly before it, so it maps
        // straight onto an EditTarget.
        handleDoubleClickOn: (_view, _pos, node, nodePos, event) => {
          const target = editTargetFromNode(node, nodePos);
          if (!target) return false;
          event.preventDefault();
          services.onEditMedia(target);
          return true;
        },
      },
      view: () => ({
        // Cheap: recompute the target from the (already-in-hand) selection and
        // only notify when its identity actually changes.
        update: (view) => {
          const target = detectEditTarget(view.state.selection);
          const key = targetKey(target);
          if (key === lastKey) return;
          lastKey = key;
          services.onTargetChange(target);
        },
      }),
    });
  });
}
