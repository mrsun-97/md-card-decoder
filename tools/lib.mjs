// Shared helpers for the Node build tools.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

export const root = path.resolve(import.meta.dirname, '..');
export const toolsBin = path.join(root, '.tools', 'bin');

/** Run a command, streaming output; throws on failure. */
export function run(cmd, args, opts = {}) {
  const env = { ...process.env, PATH: `${toolsBin}${path.delimiter}${process.env.PATH}` };
  execFileSync(cmd, args, { stdio: 'inherit', cwd: root, env, ...opts });
}

/** Run a command and return trimmed stdout, or null if it cannot be run. */
export function capture(cmd, args) {
  const env = { ...process.env, PATH: `${toolsBin}${path.delimiter}${process.env.PATH}` };
  try {
    return execFileSync(cmd, args, { cwd: root, env, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
  } catch {
    return null;
  }
}

/** Version of a crate as resolved in Cargo.lock. */
export function lockedVersion(crate) {
  const lock = fs.readFileSync(path.join(root, 'Cargo.lock'), 'utf8');
  const m = lock.match(new RegExp(`\\[\\[package\\]\\]\\nname = "${crate}"\\nversion = "([^"]+)"`));
  return m ? m[1] : null;
}

/**
 * Launch headless Chromium for tests. If the system lacks libasound (ALSA), a stub built by
 * `node tools/build-alsa-stub.mjs` in .tools/stub-alsa is put on LD_LIBRARY_PATH (audio is never used).
 */
export async function launchBrowser() {
  const { chromium } = await import('playwright');
  const stub = path.join(root, '.tools', 'stub-alsa');
  const env = { ...process.env };
  if (fs.existsSync(path.join(stub, 'libasound.so.2'))) {
    env.LD_LIBRARY_PATH = [stub, process.env.LD_LIBRARY_PATH].filter(Boolean).join(path.delimiter);
  }
  // Under WSL without CJK fonts, borrow the Windows fonts so screenshots show Chinese text.
  const winFonts = '/mnt/c/Windows/Fonts';
  if (fs.existsSync(path.join(winFonts, 'msyh.ttc'))) {
    const conf = path.join(root, '.tools', 'fonts.conf');
    fs.mkdirSync(path.dirname(conf), { recursive: true });
    fs.writeFileSync(conf, `<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "fonts.dtd"><fontconfig><include ignore_missing="yes">/etc/fonts/fonts.conf</include><dir>${winFonts}</dir><cachedir>${path.join(root, '.tools', 'fontcache')}</cachedir></fontconfig>`);
    env.FONTCONFIG_FILE = conf;
  }
  return chromium.launch({ env });
}
