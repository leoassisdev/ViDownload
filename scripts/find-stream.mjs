#!/usr/bin/env node
/**
 * Abre uma URL no Chrome real via CDP, intercepta todo tráfego de rede,
 * e retorna URLs de streams HLS/m3u8 encontrados.
 *
 * Uso: node scripts/find-stream.mjs <url> [timeout_seconds]
 * Saída: JSON com array de URLs encontrados
 */

import puppeteer from 'puppeteer-core';
import { execSync } from 'child_process';
import path from 'path';

const url = process.argv[2];
const timeout = (parseInt(process.argv[3]) || 30) * 1000;

if (!url) {
  console.error(JSON.stringify({ error: 'URL não fornecida' }));
  process.exit(1);
}

// Encontrar Chrome no sistema
function findChrome() {
  const paths = [
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/Applications/Chromium.app/Contents/MacOS/Chromium',
    '/usr/bin/google-chrome',
    '/usr/bin/chromium-browser',
  ];
  for (const p of paths) {
    try {
      execSync(`test -f "${p}"`);
      return p;
    } catch {}
  }
  // Tentar which
  try {
    return execSync('which google-chrome || which chromium').toString().trim();
  } catch {}
  return null;
}

const chromePath = findChrome();
if (!chromePath) {
  console.error(JSON.stringify({ error: 'Chrome não encontrado no sistema' }));
  process.exit(1);
}

const detected = new Map(); // url -> { url, type, contentType }

try {
  const browser = await puppeteer.launch({
    executablePath: chromePath,
    headless: false, // Visível para o usuário interagir se necessário
    args: [
      '--no-first-run',
      '--no-default-browser-check',
      `--window-size=1200,800`,
    ],
    defaultViewport: null,
  });

  const page = (await browser.pages())[0] || await browser.newPage();

  // Interceptar TODAS as respostas de rede
  page.on('response', async (response) => {
    const respUrl = response.url();
    const ct = response.headers()['content-type'] || '';
    const status = response.status();

    if (status < 200 || status >= 400) return;

    const isStream =
      /\.(m3u8|m3u|mpd)(\?|$)/i.test(respUrl) ||
      /mpegurl|dash\+xml/i.test(ct) ||
      /manifest.*format.*m3u/i.test(respUrl);

    if (isStream && !detected.has(respUrl)) {
      // Tentar determinar o tipo
      let type = 'unknown';
      if (/m3u8|m3u|mpegurl/i.test(respUrl + ct)) type = 'hls';
      else if (/mpd|dash/i.test(respUrl + ct)) type = 'dash';

      detected.set(respUrl, {
        url: respUrl,
        type,
        contentType: ct,
      });

      // Imprimir stream encontrado imediatamente (cada linha é um JSON)
      console.log(JSON.stringify({ stream: { url: respUrl, type, contentType: ct } }));
    }
  });

  // Navegar
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 15000 }).catch(() => {});

  // Esperar streams aparecerem (o vídeo pode demorar para carregar)
  const start = Date.now();
  while (Date.now() - start < timeout) {
    if (detected.size > 0) {
      // Esperar mais 3s por streams adicionais (qualidades diferentes)
      await new Promise(r => setTimeout(r, 3000));
      break;
    }
    await new Promise(r => setTimeout(r, 500));
  }

  // Resultado final
  const streams = Array.from(detected.values());
  console.log(JSON.stringify({ done: true, streams }));

  await browser.close();
} catch (err) {
  console.error(JSON.stringify({ error: err.message }));
  process.exit(1);
}
