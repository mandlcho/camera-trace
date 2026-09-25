import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createReadStream } from 'node:fs';
import { stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { dirname, extname, join } from 'node:path';
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
const dist = new URL('../dist/', import.meta.url).pathname;
const contentTypes = {
  '.css': 'text/css',
  '.html': 'text/html',
  '.js': 'text/javascript',
  '.wasm': 'application/wasm',
};

const server = createServer(async (request, response) => {
  const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
    .replace(/^\/camera-trace\/?/, '');
  const path = join(dist, pathname || 'index.html');
  try {
    const info = await stat(path);
    if (!info.isFile()) throw new Error('Not a file');
    response.writeHead(200, { 'content-type': contentTypes[extname(path)] || 'application/octet-stream' });
    createReadStream(path).pipe(response);
  } catch {
    response.writeHead(404).end('Not found');
  }
});

await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const baseUrl = `http://127.0.0.1:${server.address().port}/camera-trace/`;
test.after(() => server.close());

const onePixelPng = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M/wHwAF/gL+Xw4AAAAASUVORK5CYII=',
  'base64',
);

async function importPhoto(page) {
  await page.locator('input[type=file]').setInputFiles({
    name: 'reference.png',
    mimeType: 'image/png',
    buffer: onePixelPng,
  });
  await page.getByRole('button', { name: 'Replace photo' }).waitFor();
}

function collectConsoleErrors(page) {
  const errors = [];
  page.on('console', message => {
    if (message.type() === 'error') errors.push(message.text());
  });
  page.on('pageerror', error => errors.push(error.message));
  return errors;
}

test('fresh visit, cancelled import, skip, reload, and help focus behavior', async () => {
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 320, height: 568 } });
    const consoleErrors = collectConsoleErrors(page);
    await page.goto(baseUrl);
    await page.getByRole('heading', { name: 'Trace a photo onto paper' }).waitFor();
    assert.equal(await page.getByRole('button', { name: 'Flip' }).count(), 0);

    const chooser = page.waitForEvent('filechooser');
    await page.getByRole('button', { name: 'Choose photo' }).click();
    await (await chooser).setFiles([]);
    assert.equal(await page.evaluate(() => localStorage.getItem('camera-trace:onboarding:v1')), null);
    await page.getByRole('heading', { name: 'Trace a photo onto paper' }).waitFor();

    await page.getByRole('button', { name: 'Skip intro' }).click();
    assert.equal(
      await page.evaluate(() => localStorage.getItem('camera-trace:onboarding:v1')),
      'dismissed',
    );
    await page.reload();
    assert.equal(await page.getByRole('heading', { name: 'Trace a photo onto paper' }).count(), 0);

    const guideTrigger = page.getByRole('button', { name: 'Setup guide' });
    await guideTrigger.click();
    await page.getByRole('dialog', { name: 'Setup guide' }).waitFor();
    assert.equal(await page.evaluate(() => document.activeElement?.getAttribute('aria-label')), 'Close setup guide');
    await page.keyboard.press('Escape');
    assert.equal(await page.getByRole('dialog').count(), 0);
    assert.equal(await guideTrigger.evaluate(element => element === document.activeElement), true);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    assert.deepEqual(consoleErrors, []);
  } finally {
    await browser.close();
  }
});

test('failed import keeps onboarding open and successful import preserves workspace through help', async () => {
  const browser = await chromium.launch({
    headless: true,
    args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'],
  });
  try {
    const context = await browser.newContext({
      viewport: { width: 390, height: 844 },
      permissions: ['camera'],
    });
    const page = await context.newPage();
    const consoleErrors = collectConsoleErrors(page);
    await page.goto(baseUrl);
    await page.getByRole('heading', { name: 'Trace a photo onto paper' }).waitFor();

    await page.locator('input[type=file]').setInputFiles({
      name: 'broken.png',
      mimeType: 'image/png',
      buffer: Buffer.from('not an image'),
    });
    await page.getByRole('alert').waitFor();
    assert.equal(await page.evaluate(() => localStorage.getItem('camera-trace:onboarding:v1')), null);

    await importPhoto(page);
    assert.equal(
      await page.evaluate(() => localStorage.getItem('camera-trace:onboarding:v1')),
      'dismissed',
    );
    await page.getByRole('button', { name: 'Start camera' }).click();
    await page.locator('.context-hint').waitFor();

    const transformBeforeDrag = await page.locator('img.reference').getAttribute('style');
    const gestureBox = await page.locator('.gesture-surface').boundingBox();
    assert.ok(gestureBox);
    const dragX = gestureBox.x + gestureBox.width / 2;
    const dragY = gestureBox.y + Math.min(300, gestureBox.height / 3);
    assert.equal(
      await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.className, { x: dragX, y: dragY }),
      'gesture-surface',
    );
    const gestureSurface = page.locator('.gesture-surface');
    await gestureSurface.dispatchEvent('pointerdown', {
      clientX: dragX,
      clientY: dragY,
      pointerId: 1,
      bubbles: true,
    });
    await gestureSurface.dispatchEvent('pointermove', {
      clientX: dragX + 35,
      clientY: dragY + 25,
      pointerId: 1,
      bubbles: true,
    });
    await gestureSurface.dispatchEvent('pointerup', {
      clientX: dragX + 35,
      clientY: dragY + 25,
      pointerId: 1,
      bubbles: true,
    });
    await page.waitForTimeout(50);
    assert.notEqual(await page.locator('img.reference').getAttribute('style'), transformBeforeDrag);
    await page.getByRole('button', { name: 'Dismiss alignment tip' }).click();
    assert.equal(await page.locator('.context-hint').count(), 0);
    assert.equal(
      await page.evaluate(() => localStorage.getItem('camera-trace:alignment-hint:v1')),
      'dismissed',
    );

    const opacity = page.getByLabel('Opacity');
    await opacity.evaluate(input => {
      input.value = '0.73';
      input.dispatchEvent(new InputEvent('input', { bubbles: true }));
    });
    const sourceBefore = await page.locator('img.reference').getAttribute('src');
    await page.getByRole('button', { name: 'Lock position' }).click();
    assert.equal(await page.locator('.context-hint').count(), 0);

    await page.getByRole('button', { name: 'Setup guide' }).click();
    await page.getByRole('button', { name: 'Close setup guide' }).click();
    assert.equal(await page.getByRole('button', { name: 'Unlock' }).count(), 1);
    assert.equal(await opacity.inputValue(), '0.73');
    assert.equal(await page.locator('img.reference').getAttribute('src'), sourceBefore);
    assert.deepEqual(consoleErrors, []);
  } finally {
    await browser.close();
  }
});

test('camera denial preserves the photo and offers retry', async () => {
  const browser = await chromium.launch({ headless: true });
  try {
    const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
    await context.addInitScript(() => {
      Object.defineProperty(navigator, 'mediaDevices', {
        configurable: true,
        value: { getUserMedia: () => Promise.reject(new DOMException('Denied', 'NotAllowedError')) },
      });
    });
    const page = await context.newPage();
    const consoleErrors = collectConsoleErrors(page);
    await page.goto(baseUrl);
    await importPhoto(page);
    await page.getByRole('button', { name: 'Start camera' }).click();
    await page.getByText('Camera access was denied.').waitFor();
    assert.equal(await page.getByRole('button', { name: 'Replace photo' }).count(), 1);
    assert.equal(await page.locator('img.reference').count(), 1);
    assert.equal(await page.getByRole('button', { name: 'Start camera' }).count(), 1);
    await page.getByRole('button', { name: 'Start camera' }).click();
    await page.getByText('Camera access was denied.').waitFor();
    assert.deepEqual(consoleErrors, []);
  } finally {
    await browser.close();
  }
});

test('storage failures fall back safely and layouts remain scrollable without overflow', async () => {
  const browser = await chromium.launch({ headless: true });
  try {
    const context = await browser.newContext({ viewport: { width: 568, height: 320 } });
    await context.addInitScript(() => {
      Storage.prototype.getItem = () => { throw new DOMException('Unavailable', 'SecurityError'); };
      Storage.prototype.setItem = () => { throw new DOMException('Unavailable', 'SecurityError'); };
    });
    const page = await context.newPage();
    const consoleErrors = collectConsoleErrors(page);
    await page.goto(baseUrl);
    await page.getByRole('heading', { name: 'Trace a photo onto paper' }).waitFor();
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    assert.equal(
      await page.locator('.intro-screen').evaluate(element => element.scrollHeight > element.clientHeight),
      true,
    );
    const actionSize = await page.getByRole('button', { name: 'Choose photo' }).boundingBox();
    assert.ok(actionSize.height >= 44);
    assert.deepEqual(consoleErrors, []);
  } finally {
    await browser.close();
  }
});
