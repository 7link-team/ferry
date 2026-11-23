import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { inflateSync, deflateSync } from 'node:zlib';

export function decodePng(bytes) {
  const compressed = [];
  let header;
  for (let offset = 8; offset < bytes.length;) {
    const length = bytes.readUInt32BE(offset), type = bytes.toString('ascii', offset + 4, offset + 8);
    if (type === 'IHDR') header = bytes.subarray(offset + 8, offset + 8 + length);
    if (type === 'IDAT') compressed.push(bytes.subarray(offset + 8, offset + 8 + length));
    offset += length + 12;
  }
  if (!header || header[8] !== 8 || header[9] !== 6 || header[12] !== 0) throw new Error('Expected RGBA8 PNG artwork');
  const width = header.readUInt32BE(0), height = header.readUInt32BE(4), stride = width * 4;
  const raw = inflateSync(Buffer.concat(compressed)), pixels = Buffer.alloc(stride * height);
  if (raw.length !== (stride + 1) * height) throw new Error('Invalid PNG scanline size');
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    if (filter > 4) throw new Error('Invalid PNG filter');
    for (let x = 0; x < stride; x++) {
      const offset = y * stride + x, left = x >= 4 ? pixels[offset - 4] : 0;
      const up = y ? pixels[offset - stride] : 0, corner = y && x >= 4 ? pixels[offset - stride - 4] : 0;
      const p = left + up - corner, dl = Math.abs(p - left), du = Math.abs(p - up), dc = Math.abs(p - corner);
      const prediction = filter === 0 ? 0 : filter === 1 ? left : filter === 2 ? up
        : filter === 3 ? Math.floor((left + up) / 2) : dl <= du && dl <= dc ? left : du <= dc ? up : corner;
      pixels[offset] = (raw[y * (stride + 1) + 1 + x] + prediction) & 255;
    }
  }
  return { width, height, pixels };
}

function chunk(type, data) {
  const name = Buffer.from(type);
  let crc = 0xffffffff;
  for (const byte of Buffer.concat([name, data])) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
  }
  const prefix = Buffer.alloc(4), suffix = Buffer.alloc(4);
  prefix.writeUInt32BE(data.length); suffix.writeUInt32BE((crc ^ 0xffffffff) >>> 0);
  return Buffer.concat([prefix, name, data, suffix]);
}

export function encodePng({ width, height, pixels }) {
  const header = Buffer.alloc(13), stride = width * 4, raw = Buffer.alloc((stride + 1) * height);
  header.writeUInt32BE(width); header.writeUInt32BE(height, 4); header[8] = 8; header[9] = 6;
  for (let y = 0; y < height; y++) pixels.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  return Buffer.concat([Buffer.from('89504e470d0a1a0a', 'hex'), chunk('IHDR', header),
    chunk('IDAT', deflateSync(raw, { level: 9 })), chunk('IEND', Buffer.alloc(0))]);
}

// Integer overlap weights and premultiplied colors avoid platform-dependent
// floating-point resampling and colored fringes around transparent artwork.
export function resizeArea(source, width, height = width) {
  const pixels = Buffer.alloc(width * height * 4), area = source.width * source.height;
  for (let y = 0; y < height; y++) {
    const top = y * source.height, bottom = (y + 1) * source.height;
    for (let x = 0; x < width; x++) {
      const left = x * source.width, right = (x + 1) * source.width;
      let alpha = 0, red = 0, green = 0, blue = 0;
      for (let sy = Math.floor(top / height); sy < Math.ceil(bottom / height); sy++) {
        const wy = Math.min(bottom, (sy + 1) * height) - Math.max(top, sy * height);
        for (let sx = Math.floor(left / width); sx < Math.ceil(right / width); sx++) {
          const weight = wy * (Math.min(right, (sx + 1) * width) - Math.max(left, sx * width));
          const offset = (sy * source.width + sx) * 4, wa = weight * source.pixels[offset + 3];
          alpha += wa; red += wa * source.pixels[offset]; green += wa * source.pixels[offset + 1]; blue += wa * source.pixels[offset + 2];
        }
      }
      const offset = (y * width + x) * 4;
      if (alpha) {
        pixels[offset] = Math.floor((red + Math.floor(alpha / 2)) / alpha);
        pixels[offset + 1] = Math.floor((green + Math.floor(alpha / 2)) / alpha);
        pixels[offset + 2] = Math.floor((blue + Math.floor(alpha / 2)) / alpha);
      }
      pixels[offset + 3] = Math.floor((alpha + Math.floor(area / 2)) / area);
    }
  }
  return { width, height, pixels };
}

function opaqueWhite(image) {
  const pixels = Buffer.from(image.pixels);
  for (let offset = 0; offset < pixels.length; offset += 4) {
    const alpha = pixels[offset + 3];
    for (let c = 0; c < 3; c++) pixels[offset + c] = Math.floor((pixels[offset + c] * alpha + 255 * (255 - alpha) + 127) / 255);
    pixels[offset + 3] = 255;
  }
  return { ...image, pixels };
}

function androidMask(image, rounded) {
  const size = image.width;
  // Same margins and clipping contract as the pinned Tauri icon generator.
  const margin = rounded ? Math.floor((size * 4 + 50) / 100) : Math.floor((size * 833 + 5000) / 10000);
  const innerSize = size - margin * 2, radius = Math.min(rounded ? Math.ceil(size / 2) : margin, Math.floor(innerSize / 2));
  const inner = resizeArea(image, innerSize), pixels = Buffer.alloc(size * size * 4);
  for (let y = margin; y < size - margin; y++) {
    for (let x = margin; x < size - margin; x++) {
      const cx = Math.max(margin + radius, Math.min(x, size - margin - radius));
      const cy = Math.max(margin + radius, Math.min(y, size - margin - radius));
      if ((x - cx) ** 2 + (y - cy) ** 2 > radius * radius) continue;
      const from = ((y - margin) * innerSize + x - margin) * 4;
      inner.pixels.copy(pixels, (y * size + x) * 4, from, from + 4);
    }
  }
  return { width: size, height: size, pixels };
}

function entries(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? entries(path) : [path];
  });
}

export function normalizeNativeIconImages(directory, master) {
  const source = decodePng(master), cache = new Map();
  const image = size => {
    if (!cache.has(size)) cache.set(size, resizeArea(source, size));
    return cache.get(size);
  };
  for (const path of entries(directory)) {
    if (!path.endsWith('.png')) continue;
    const old = readFileSync(path), size = old.readUInt32BE(16), name = relative(directory, path).replaceAll('\\', '/');
    let result = image(size);
    if (name.startsWith('ios/')) result = opaqueWhite(result);
    else if (name.startsWith('android/') && /\/ic_launcher(?:_round)?\.png$/.test(name)) result = androidMask(result, name.endsWith('_round.png'));
    writeFileSync(path, encodePng(result));
  }
  const icoPath = join(directory, 'icon.ico'), oldIco = readFileSync(icoPath), count = oldIco.readUInt16LE(4);
  const icoHeader = Buffer.alloc(6 + count * 16), icoImages = [];
  icoHeader.writeUInt16LE(1, 2); icoHeader.writeUInt16LE(count, 4);
  let offset = icoHeader.length;
  for (let i = 0; i < count; i++) {
    const size = oldIco[6 + i * 16] || 256, bytes = encodePng(image(size)), entry = 6 + i * 16;
    icoHeader[entry] = size === 256 ? 0 : size; icoHeader[entry + 1] = icoHeader[entry];
    icoHeader.writeUInt16LE(1, entry + 4); icoHeader.writeUInt16LE(32, entry + 6);
    icoHeader.writeUInt32LE(bytes.length, entry + 8); icoHeader.writeUInt32LE(offset, entry + 12);
    icoImages.push(bytes); offset += bytes.length;
  }
  writeFileSync(icoPath, Buffer.concat([icoHeader, ...icoImages]));
  const icnsPath = join(directory, 'icon.icns'), oldIcns = readFileSync(icnsPath), parts = [];
  for (let start = 8; start < oldIcns.length;) {
    const type = oldIcns.toString('ascii', start, start + 4), length = oldIcns.readUInt32BE(start + 4);
    let payload;
    if (['is32', 'il32', 's8mk', 'l8mk'].includes(type)) {
      const size = ['is32', 's8mk'].includes(type) ? 16 : 32, { pixels } = image(size);
      if (type.endsWith('mk')) payload = Buffer.from(Array.from({ length: size * size }, (_, i) => pixels[i * 4 + 3]));
      else {
        const channels = [];
        for (let channel = 0; channel < 3; channel++) {
          const data = Buffer.from(Array.from({ length: size * size }, (_, i) => pixels[i * 4 + channel]));
          // ICNS RGB uses PackBits; literal runs preserve every byte exactly.
          for (let i = 0; i < data.length; i += 128) {
            const run = data.subarray(i, i + 128); channels.push(Buffer.from([run.length - 1]), run);
          }
        }
        payload = Buffer.concat(channels);
      }
    } else {
      const size = oldIcns.readUInt32BE(start + 8 + 16);
      payload = encodePng(image(size));
    }
    const head = Buffer.alloc(8); head.write(type); head.writeUInt32BE(payload.length + 8, 4);
    parts.push(Buffer.concat([head, payload])); start += length;
  }
  parts.sort(Buffer.compare);
  const head = Buffer.alloc(8); head.write('icns'); head.writeUInt32BE(8 + parts.reduce((sum, part) => sum + part.length, 0), 4);
  writeFileSync(icnsPath, Buffer.concat([head, ...parts]));
}
