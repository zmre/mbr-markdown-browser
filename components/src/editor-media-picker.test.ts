import { describe, expect, it } from 'vitest';
import { buildMediaOutput, toVidPath } from './editor-media-picker.js';

describe('toVidPath', () => {
  it('strips the /videos/ prefix', () => {
    expect(toVidPath('/videos/demo.mp4')).toBe('demo.mp4');
    expect(toVidPath('/videos/Eric Jones/clip.mp4')).toBe('Eric Jones/clip.mp4');
  });

  it('strips a leading slash for non-/videos/ paths', () => {
    expect(toVidPath('/media/clip.mp4')).toBe('media/clip.mp4');
  });
});

describe('buildMediaOutput', () => {
  it('emits an image embed for images', () => {
    expect(
      buildMediaOutput({ url: '/images/photo.jpg', kind: 'image', caption: 'A photo' }),
    ).toEqual({ form: 'image', src: '/images/photo.jpg', caption: 'A photo' });
  });

  it('emits an image embed for audio and pdf', () => {
    expect(buildMediaOutput({ url: '/audio/song.mp3', kind: 'audio', caption: '' })).toEqual({
      form: 'image',
      src: '/audio/song.mp3',
      caption: '',
    });
    expect(buildMediaOutput({ url: '/docs/spec.pdf', kind: 'pdf', caption: 'Spec' })).toEqual({
      form: 'image',
      src: '/docs/spec.pdf',
      caption: 'Spec',
    });
  });

  it('emits an image embed for a video without timestamps', () => {
    expect(
      buildMediaOutput({ url: '/videos/demo.mp4', kind: 'video', caption: 'Demo' }),
    ).toEqual({ form: 'image', src: '/videos/demo.mp4', caption: 'Demo' });
  });

  it('emits a vid shortcode for a video with a start', () => {
    const out = buildMediaOutput({
      url: '/videos/demo.mp4',
      kind: 'video',
      caption: 'Demo',
      start: '10',
    });
    expect(out).toEqual({
      form: 'shortcode',
      shortcode: '{{ vid(path="demo.mp4", start="10", caption="Demo") }}',
    });
  });

  it('emits a vid shortcode with start and end', () => {
    const out = buildMediaOutput({
      url: '/videos/demo.mp4',
      kind: 'video',
      caption: '',
      start: '10',
      end: '30',
    });
    expect(out).toEqual({
      form: 'shortcode',
      shortcode: '{{ vid(path="demo.mp4", start="10", end="30") }}',
    });
  });

  it('emits a vid shortcode with only an end', () => {
    const out = buildMediaOutput({
      url: '/videos/demo.mp4',
      kind: 'video',
      caption: 'Clip',
      end: '30',
    });
    expect(out).toEqual({
      form: 'shortcode',
      shortcode: '{{ vid(path="demo.mp4", end="30", caption="Clip") }}',
    });
  });

  it('trims whitespace from caption and timestamps', () => {
    const out = buildMediaOutput({
      url: '/videos/demo.mp4',
      kind: 'video',
      caption: '  Demo  ',
      start: ' 10 ',
    });
    expect(out).toEqual({
      form: 'shortcode',
      shortcode: '{{ vid(path="demo.mp4", start="10", caption="Demo") }}',
    });
  });
});
