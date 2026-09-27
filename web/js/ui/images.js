// Card thumbnails. Low-quality WebP thumbnails are packed into sprite sheets (≤ 1,000 files for a
// dashboard upload); window.CARD_SHEETS = { cols, rows, w, h, files: [...], cell: [sheet*cells+i or -1] }.
// Without sheets (or for an unknown card) a CSS card is drawn from the card's frame colours.
import { escapeHtml, frameGradient } from '../labels.js';

export function thumbHtml(data, g, { zoom = false, small = false } = {}) {
  const cls = `card-thumb${small ? ' small' : ''}`;
  if (g == null) return `<div class="${cls} back" aria-label="未知卡片"><span>?</span></div>`;
  const sheets = window.CARD_SHEETS;
  const z = zoom ? ` data-zoom="${g}" role="button" tabindex="0"` : '';
  const title = ` title="${escapeHtml(data.name(g))}"`;
  const cell = sheets?.cell?.[g] ?? -1;
  if (sheets && cell >= 0) {
    const per = sheets.cols * sheets.rows;
    const file = sheets.files[Math.floor(cell / per)];
    const i = cell % per;
    const x = (100 * (i % sheets.cols)) / (sheets.cols - 1);
    const y = (100 * Math.floor(i / sheets.cols)) / (sheets.rows - 1);
    return `<div class="${cls} img"${z}${title} style="background-image:url('${file}');background-size:${sheets.cols * 100}% ${sheets.rows * 100}%;background-position:${x}% ${y}%"></div>`;
  }
  const name = data.name(g);
  return `<div class="${cls} drawn"${z}${title} style="--frame-fill:${frameGradient(data.value(g, 0))}"><span>${escapeHtml(name.slice(0, 6))}</span></div>`;
}

export function openViewer(data, g) {
  const dlg = document.querySelector('#imageViewer');
  document.querySelector('#imageViewerThumb').innerHTML = thumbHtml(data, g);
  document.querySelector('#imageViewerCaption').textContent = `${data.name(g)} · 卡片密码 ${data.passcode(g)}`;
  dlg.showModal();
}
