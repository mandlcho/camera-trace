import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname } from 'node:path';
import test from 'node:test';

const require = createRequire(import.meta.url);

function loadPlaywright() {
  try {
    return require('playwright');
  } catch {
    const executable = execFileSync('which', ['playwright'], { encoding: 'utf8' }).trim();
    const cli = execFileSync('realpath', [executable], { encoding: 'utf8' }).trim();
    return require(dirname(cli));
  }
}

const { chromium } = loadPlaywright();
const moduleSource = await readFile(new URL('../image-import.js', import.meta.url), 'utf8');
const styles = await readFile(new URL('../styles.css', import.meta.url), 'utf8');

test('image import preserves alpha and keeps opaque photos as JPEG', async () => {
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage();
    await page.route('http://camera-trace.test/', route => route.fulfill({
      contentType: 'text/html',
      body: '<main style="background:rgb(12, 34, 56)"></main>',
    }));
    await page.goto('http://camera-trace.test/');
    await page.addStyleTag({ content: styles });
    await page.addScriptTag({
      type: 'module',
      content: `${moduleSource}\nwindow.read_image = read_image;`,
    });
    await page.waitForFunction(() => typeof window.read_image === 'function');

    const result = await page.evaluate(async () => {
      const pixels = new Uint8ClampedArray([
        0, 0, 0, 0,     0, 0, 0, 255,
        0, 0, 0, 128,   0, 0, 0, 0,
      ]);
      const source = document.createElement('canvas');
      source.width = 2;
      source.height = 2;
      source.getContext('2d').putImageData(new ImageData(pixels, 2, 2), 0, 0);
      const pngBlob = await new Promise(resolve => source.toBlob(resolve, 'image/png'));
      const encoded = await window.read_image(new File([pngBlob], 'line.png', { type: 'image/png' }));

      const decoded = new Image();
      decoded.src = encoded;
      await decoded.decode();
      const output = document.createElement('canvas');
      output.width = 2;
      output.height = 2;
      output.getContext('2d').drawImage(decoded, 0, 0);
      const outputPixels = [...output.getContext('2d').getImageData(0, 0, 2, 2).data];

      const composite = document.createElement('canvas');
      composite.width = 2;
      composite.height = 2;
      const compositeContext = composite.getContext('2d');
      compositeContext.fillStyle = 'rgb(12, 34, 56)';
      compositeContext.fillRect(0, 0, 2, 2);
      compositeContext.drawImage(decoded, 0, 0);
      const backgroundPixel = [...compositeContext.getImageData(0, 0, 1, 1).data];

      const database = await new Promise((resolve, reject) => {
        const request = indexedDB.open('alpha-regression', 1);
        request.onupgradeneeded = () => request.result.createObjectStore('images');
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      await new Promise((resolve, reject) => {
        const transaction = database.transaction('images', 'readwrite');
        transaction.objectStore('images').put(encoded, 'drawing');
        transaction.oncomplete = resolve;
        transaction.onerror = () => reject(transaction.error);
      });
      const reloaded = await new Promise((resolve, reject) => {
        const request = database.transaction('images').objectStore('images').get('drawing');
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });

      const photo = document.createElement('canvas');
      photo.width = 2000;
      photo.height = 1000;
      photo.getContext('2d').fillRect(0, 0, 2000, 1000);
      const jpegBlob = await new Promise(resolve => photo.toBlob(resolve, 'image/jpeg'));
      const jpeg = await window.read_image(new File([jpegBlob], 'photo.jpg', { type: 'image/jpeg' }));
      const decodedJpeg = new Image();
      decodedJpeg.src = jpeg;
      await decodedJpeg.decode();

      const reference = document.createElement('img');
      reference.className = 'reference';
      document.body.append(reference);

      return {
        encoded,
        outputPixels,
        backgroundPixel,
        reloaded,
        jpeg,
        jpegSize: [decodedJpeg.naturalWidth, decodedJpeg.naturalHeight],
        blendMode: getComputedStyle(reference).mixBlendMode,
      };
    });

    assert.match(result.encoded, /^data:image\/png;base64,/);
    assert.deepEqual(result.outputPixels.slice(0, 4), [0, 0, 0, 0]);
    assert.deepEqual(result.outputPixels.slice(4, 8), [0, 0, 0, 255]);
    assert.deepEqual(result.outputPixels.slice(8, 11), [0, 0, 0]);
    assert.ok(result.outputPixels[11] >= 127 && result.outputPixels[11] <= 128);
    assert.deepEqual(result.backgroundPixel, [12, 34, 56, 255]);
    assert.equal(result.reloaded, result.encoded);
    assert.match(result.jpeg, /^data:image\/jpeg;base64,/);
    assert.deepEqual(result.jpegSize, [1800, 900]);
    assert.equal(result.blendMode, 'multiply');
  } finally {
    await browser.close();
  }
});
