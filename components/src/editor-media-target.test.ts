import { describe, expect, it } from 'vitest';
import type { Node as ProseNode } from '@milkdown/kit/prose/model';
import { editTargetFromNode, imageEditFields } from './editor-media-target.js';

describe('imageEditFields', () => {
  it('maps a block image caption from the `caption` attr', () => {
    expect(imageEditFields('image-block', { src: '/images/a.jpg', caption: 'A' })).toEqual({
      src: '/images/a.jpg',
      caption: 'A',
      typeName: 'image-block',
    });
  });

  it('maps an inline image caption from the `alt` attr', () => {
    expect(imageEditFields('image', { src: '/images/b.png', alt: 'Bee' })).toEqual({
      src: '/images/b.png',
      caption: 'Bee',
      typeName: 'image',
    });
  });

  it('defaults missing src/caption attrs to empty strings', () => {
    expect(imageEditFields('image-block', {})).toEqual({
      src: '',
      caption: '',
      typeName: 'image-block',
    });
    expect(imageEditFields('image', {})).toEqual({ src: '', caption: '', typeName: 'image' });
  });

  it('reads only the caption attr appropriate to the node type', () => {
    // A block image ignores `alt`; an inline image ignores `caption`.
    expect(imageEditFields('image-block', { src: '/x', alt: 'ignored' })!.caption).toBe('');
    expect(imageEditFields('image', { src: '/x', caption: 'ignored' })!.caption).toBe('');
  });

  it('returns null for non-image nodes', () => {
    expect(imageEditFields('paragraph', { src: '/x' })).toBeNull();
    expect(imageEditFields('text', {})).toBeNull();
  });
});

describe('editTargetFromNode', () => {
  // Minimal ProseMirror-Node-shaped fake: editTargetFromNode only reads
  // `type.name`, `attrs`, and `nodeSize`.
  const fakeNode = (name: string, attrs: Record<string, unknown>, nodeSize: number): ProseNode =>
    ({ type: { name }, attrs, nodeSize }) as unknown as ProseNode;

  it('carries position and nodeSize onto a block-image target', () => {
    expect(editTargetFromNode(fakeNode('image-block', { src: '/a.jpg', caption: 'A' }, 1), 7)).toEqual(
      { src: '/a.jpg', caption: 'A', typeName: 'image-block', pos: 7, nodeSize: 1 },
    );
  });

  it('carries position and nodeSize onto an inline-image target', () => {
    expect(editTargetFromNode(fakeNode('image', { src: '/b.png', alt: 'Bee' }, 1), 3)).toEqual({
      src: '/b.png',
      caption: 'Bee',
      typeName: 'image',
      pos: 3,
      nodeSize: 1,
    });
  });

  it('returns null for a non-image node', () => {
    expect(editTargetFromNode(fakeNode('paragraph', {}, 2), 0)).toBeNull();
  });
});
