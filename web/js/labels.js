// Field metadata and value formatting, using the in-game wording (火 not 炎, races with 族).

export const FIELDS = [
  { key: 'frame', bit: 1, label: '卡片边框', short: '边框' },
  { key: 'attribute', bit: 2, label: '属性', short: '属性' },
  { key: 'race', bit: 4, label: '种族', short: '种族' },
  { key: 'level', bit: 8, label: '等级／阶级／连接', short: '等级' },
  { key: 'atk', bit: 16, label: '攻击力', short: '攻击' },
  { key: 'def', bit: 32, label: '守备力', short: '守备' },
];

const FRAME = { 1: '通常', 2: '效果', 4: '融合', 8: '同步', 16: '超量', 32: '连接', 64: '仪式', 128: '灵摆' };
const ATTRIBUTE = { 1: '地', 2: '水', 4: '火', 8: '风', 16: '光', 32: '暗', 64: '神' };
const RACE = {
  1: '战士', 2: '魔法师', 4: '天使', 8: '恶魔', 16: '不死', 32: '机械', 64: '水', 128: '炎', 256: '岩石',
  512: '鸟兽', 1024: '植物', 2048: '昆虫', 4096: '雷', 8192: '龙', 16384: '兽', 32768: '兽战士', 65536: '恐龙',
  131072: '鱼', 262144: '海龙', 524288: '爬虫类', 1048576: '念动力', 2097152: '幻神兽', 4194304: '创造神',
  8388608: '幻龙', 16777216: '电子界', 33554432: '幻想魔',
};

/** Frame parts in display order, e.g. 130 → ['效果', '灵摆']. */
export function frameParts(bits) {
  return Object.entries(FRAME).filter(([b]) => bits & Number(b)).map(([, l]) => l);
}

export function formatValue(fieldIndex, raw) {
  if (raw == null) return '未知';
  switch (FIELDS[fieldIndex].key) {
    case 'frame': return frameParts(raw).join('／');
    case 'attribute': return ATTRIBUTE[raw] ?? String(raw);
    case 'race': return (RACE[raw] ?? String(raw)) + '族';
    case 'level': return String(raw);
    default: return raw === -2 ? '?' : raw === -3 ? '—' : String(raw);
  }
}

/** Sort key for value lists (numbers numerically, labels in zh collation order). */
export function valueSortKey(fieldIndex, raw) {
  const key = FIELDS[fieldIndex].key;
  return key === 'level' || key === 'atk' || key === 'def' ? raw : formatValue(fieldIndex, raw);
}

/** Colours of frame parts for the frame chip. */
export function frameGradient(bits) {
  const colours = { 1: '#d6bd73', 2: '#c36f35', 4: '#8a55b3', 8: '#e8edf2', 16: '#2b2f3a', 32: '#315f9d', 64: '#5571ad', 128: '#3fae8f' };
  const picked = Object.entries(colours).filter(([b]) => bits & Number(b)).map(([, c]) => c);
  if (!picked.length) return '#506071';
  return picked.length === 1 ? picked[0] : `linear-gradient(135deg, ${picked[0]} 0 50%, ${picked[1]} 50% 100%)`;
}

export const escapeHtml = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

export const pct = (p, digits = 1) => `${(100 * p).toFixed(digits)}%`;

/** Normalise text for search: NFKC, lower case, no spaces or common punctuation. */
export function normalizeSearch(s) {
  return String(s).normalize('NFKC').toLowerCase().replace(/[\s·・\-—_/／「」『』－.,，。:：!！?？'"“”‘’()（）]/g, '');
}
