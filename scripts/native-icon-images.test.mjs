import { test } from 'node:test';
import assert from 'node:assert/strict';
import { decodePng, encodePng, resizeArea } from './native-icon-images.mjs';

// Independent PNG fixtures encode the same pixels using every PNG row filter.
const fixtures = [
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAF0lEQVR4nGP4z8DwHwgbGIC0w////xkAQBgHuub96GQAAAAASUVORK5CYII=",
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAGUlEQVR4nGP8z8Dwn/E/QyMjkHb4/5/hAAA/DAd/2r9BpgAAAABJRU5ErkJggg==",
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAF0lEQVR4nGP6z8DwHwgbmBgZ/jsCWQ0APd4HQVoP/fsAAAAASUVORK5CYII=",
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAG0lEQVR4nGP+z8Dwv/E/AyNzI8P/g/8bGhYAAEayCGYbXobCAAAAAElFTkSuQmCC",
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAGUlEQVR4nGP5z8Dwn/E/QyMLI8N/RyDnAAA8bQaIr+au4AAAAABJRU5ErkJggg=="
];
const pixels = Buffer.from([255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 255, 255, 255, 0]);

test('decodes all five row filters without changing RGBA values', () => {
  for (const fixture of fixtures) {
    const image = decodePng(Buffer.from(fixture, 'base64'));
    assert.equal(image.width, 2); assert.equal(image.height, 2);
    assert.deepEqual(image.pixels, pixels);
  }
});

test('resizing transparent artwork excludes invisible RGB from visible edges', () => {
  const image = { width: 2, height: 1, pixels: Buffer.from([255, 0, 0, 255, 0, 0, 255, 0]) };
  assert.deepEqual(resizeArea(image, 1, 1).pixels, Buffer.from([255, 0, 0, 128]));
  assert.deepEqual(resizeArea(image, 3, 1).pixels,
    Buffer.from([255, 0, 0, 255, 255, 0, 0, 128, 0, 0, 0, 0]));
});

test('retains exact colors and alpha in encoded icons', () => {
  const image = { width: 2, height: 2, pixels };
  assert.deepEqual(decodePng(encodePng(image)), image);
  assert.deepEqual(encodePng(image), encodePng(image));
});
