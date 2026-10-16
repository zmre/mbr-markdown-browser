/**
 * Images keep what the author wrote: untitled images are not deleted, and a
 * standalone image keeps its alt text.
 *
 * # Untitled images
 *
 * mdast gives an untitled image `title: null`, but Milkdown's `image` and
 * Crepe's `image-block` nodes declare `title`/`caption` as
 * `validate: 'string'`. ProseMirror rejects the null, Milkdown's parser logs
 * the error and moves on — and the image is simply missing from the document,
 * so the next save writes the file without it. Every `![alt](src)` was lost
 * that way; only `![alt](src "title")` survived.
 *
 * A from-markdown transform (before every remark plugin, so Crepe's
 * image-block conversion sees it too) gives such images an empty title, which
 * remark-stringify omits again on save.
 *
 * # Standalone images keep their alt text
 *
 * Crepe turns a paragraph holding only an image into its `image-block` node,
 * which keeps a display *ratio* where the alt text was: it parses
 * `Number(alt)` (1 for any real text) and saves the ratio back as the alt —
 * `![A diagram](d.png)` was saved as `![1.00](d.png)`. The schema is wrapped
 * (through its ctx slice, so Crepe's own view, uploader and commands are
 * untouched) to carry the authored `alt` too, and to print it unless the
 * ratio was actually changed in the editor (resizing), in which case the
 * ratio is written as before.
 */
import { imageBlockSchema } from '@milkdown/kit/component/image-block';
import type { Ctx, MilkdownPlugin } from '@milkdown/kit/ctx';
import type { NodeSchema } from '@milkdown/kit/transformer';
import { $syntax } from './syntax-extension.js';

interface MaybeImage {
  type: string;
  title?: string | null;
  children?: MaybeImage[];
}

function untitledImagesGetEmptyTitles(node: MaybeImage): void {
  if (node.type === 'image' && (node.title === null || node.title === undefined)) node.title = '';
  for (const child of node.children ?? []) untitledImagesGetEmptyTitles(child);
}

export const imagesSyntax = {
  fromMarkdown: {
    transforms: [(tree: unknown) => untitledImagesGetEmptyTitles(tree as MaybeImage)],
  },
};

/** Remark plugin: untitled images survive parsing. */
export const remarkMbrImages = $syntax('remarkMbrImages', imagesSyntax);

/** Crepe's reading of an image-block's alt as a display ratio. */
export function ratioFromAlt(alt: string): number {
  const ratio = Number(alt || 1);
  return Number.isNaN(ratio) || ratio === 0 ? 1 : ratio;
}

/** `image-block`'s schema, carrying and printing the authored alt text. */
function withAuthoredAlt(spec: NodeSchema): NodeSchema {
  return {
    ...spec,
    attrs: { ...spec.attrs, alt: { default: '' } },
    parseMarkdown: {
      match: spec.parseMarkdown.match,
      runner: (state, node, type) => {
        const alt = String(node.alt ?? '');
        state.addNode(type, {
          src: String(node.url ?? ''),
          caption: String(node.title ?? ''),
          ratio: ratioFromAlt(alt),
          alt,
        });
      },
    },
    toMarkdown: {
      match: spec.toMarkdown.match,
      runner: (state, node) => {
        const alt = String(node.attrs.alt ?? '');
        const ratio = Number(node.attrs.ratio);
        const resized = ratio !== ratioFromAlt(alt);
        state.openNode('paragraph');
        state.addNode('image', undefined, undefined, {
          title: node.attrs.caption,
          url: node.attrs.src,
          alt: resized ? ratio.toFixed(2) : alt,
        });
        state.closeNode();
      },
    },
  };
}

/**
 * Wraps the `image-block` schema in its ctx slice. Runs at prepare time,
 * after Crepe's ImageBlock feature injected the slice (features are `use`d in
 * Crepe's constructor, before `MARKDOWN_EXTENSION_PLUGINS`) and before the
 * node is built from it; without that feature there is nothing to wrap.
 */
const imageBlockKeepsAlt: MilkdownPlugin = (ctx: Ctx) => {
  try {
    ctx.update(imageBlockSchema.key, (previous) => (inner) => withAuthoredAlt(previous(inner)));
  } catch {
    // ImageBlock feature disabled: no slice, no image-block node.
  }
  return () => undefined;
};

/** Everything the editor needs so images survive a save as authored. */
export const imagesPlugin = [remarkMbrImages, imageBlockKeepsAlt].flat();
