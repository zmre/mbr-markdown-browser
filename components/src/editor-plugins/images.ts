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
 *
 * # Standalone images keep their place
 *
 * Crepe's conversion builds the `image-block` from the image's `url`, `alt`
 * and `title` alone, so it has no source position: nothing could tell where
 * it was, and `authored-style.ts` could not keep the blank lines around it
 * (a `---` right above one gained a blank line). The from-markdown transform
 * notes the position of every paragraph Crepe will convert, in document
 * order, and {@link remarkImageBlockPositions} gives each `image-block` its
 * paragraph's position back; the node then carries `mbrStyle` like the
 * paragraph it replaced.
 */
import { imageBlockSchema } from '@milkdown/kit/component/image-block';
import type { Ctx, MilkdownPlugin } from '@milkdown/kit/ctx';
import type { NodeSchema } from '@milkdown/kit/transformer';
import { $remark } from '@milkdown/kit/utils';
import { $syntax } from './syntax-extension.js';

interface MaybeImage {
  type: string;
  title?: string | null;
  children?: MaybeImage[];
  position?: unknown;
  data?: { mbrImageBlockPositions?: unknown[] };
}

function untitledImagesGetEmptyTitles(node: MaybeImage): void {
  if (node.type === 'image' && (node.title === null || node.title === undefined)) node.title = '';
  for (const child of node.children ?? []) untitledImagesGetEmptyTitles(child);
}

/** Whether Crepe turns `node` into an `image-block`: a paragraph holding one image. */
function isImageParagraph(node: MaybeImage): boolean {
  return node.type === 'paragraph' && node.children?.length === 1 && node.children[0].type === 'image';
}

/** Preorder, as Crepe's `visit` converts them. */
function eachNode(node: MaybeImage, f: (node: MaybeImage) => void): void {
  f(node);
  for (const child of node.children ?? []) eachNode(child, f);
}

/** Notes the positions of the paragraphs Crepe will convert, on the root. */
function noteImageParagraphs(tree: MaybeImage): void {
  const positions: unknown[] = [];
  eachNode(tree, (node) => {
    if (isImageParagraph(node)) positions.push(node.position);
  });
  tree.data = { ...tree.data, mbrImageBlockPositions: positions };
}

/** Gives every `image-block` the position of the paragraph it replaced (see the module docs). */
export function restoreImageBlockPositions(tree: MaybeImage): void {
  const positions = tree.data?.mbrImageBlockPositions;
  if (!positions) return;
  const blocks: MaybeImage[] = [];
  eachNode(tree, (node) => {
    if (node.type === 'image-block') blocks.push(node);
  });
  // Anything else made image-blocks (or some paragraphs were left): no guessing.
  if (blocks.length !== positions.length) return;
  blocks.forEach((block, i) => {
    if (block.position === undefined && positions[i] !== undefined) block.position = positions[i];
  });
}

export const imagesSyntax = {
  fromMarkdown: {
    transforms: [
      (tree: unknown) => {
        untitledImagesGetEmptyTitles(tree as MaybeImage);
        noteImageParagraphs(tree as MaybeImage);
      },
    ],
  },
};

/** After Crepe's conversion: image-blocks get their positions back. */
export const remarkImageBlockPositions = $remark('mbrImageBlockPositions', () => () => (tree: unknown) => {
  restoreImageBlockPositions(tree as MaybeImage);
});

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
    // `mbrStyle`: what `authored-style.ts` recorded for it (blank lines around it).
    attrs: { ...spec.attrs, alt: { default: '' }, mbrStyle: { default: null } },
    parseMarkdown: {
      match: spec.parseMarkdown.match,
      runner: (state, node, type) => {
        const alt = String(node.alt ?? '');
        state.addNode(type, {
          src: String(node.url ?? ''),
          caption: String(node.title ?? ''),
          ratio: ratioFromAlt(alt),
          alt,
          mbrStyle: (node as { mbrStyle?: unknown }).mbrStyle ?? null,
        });
      },
    },
    toMarkdown: {
      match: spec.toMarkdown.match,
      runner: (state, node) => {
        const alt = String(node.attrs.alt ?? '');
        const ratio = Number(node.attrs.ratio);
        const resized = ratio !== ratioFromAlt(alt);
        const style: unknown = node.attrs.mbrStyle;
        state.openNode('paragraph', undefined, style == null ? undefined : ({ mbrStyle: style } as never));
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
export const imagesPlugin = [remarkMbrImages, remarkImageBlockPositions, imageBlockKeepsAlt].flat();
