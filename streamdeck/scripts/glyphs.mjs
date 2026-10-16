// The plugin's glyphs: simple flat shapes on a 144 unit square, drawn for
// Platter. Each takes a colour and gives SVG elements. Used by
// scripts/icons.mjs for the action list and by src/render.ts for the keys.

const S = (c, w = 8) => `fill="none" stroke="${c}" stroke-width="${w}" stroke-linecap="round" stroke-linejoin="round"`;
const heart = "M72 112 34 74a22 22 0 0 1 38-30 22 22 0 0 1 38 30z";
const star = "M72 30l12.3 25 27.6 4-20 19.5 4.7 27.5L72 93l-24.6 13 4.7-27.5-20-19.5 27.6-4z";
const speaker = (c) => `<path d="M28 58h18l24-20v68L46 86H28z" fill="${c}"/>`;
const lines = (c, n = 3, w = 64) =>
  Array.from({ length: n }, (_, i) => `<rect x="30" y="${40 + i * 22}" width="${w}" height="9" rx="4.5" fill="${c}"/>`).join("");

/** name -> (colour) => svg elements */
export const GLYPHS = {
  play: (c) => `<path d="M52 36v72l58-36z" fill="${c}"/>`,
  pause: (c) => `<rect x="42" y="38" width="22" height="68" rx="5" fill="${c}"/><rect x="80" y="38" width="22" height="68" rx="5" fill="${c}"/>`,
  next: (c) => `<path d="M38 40v64l50-32z" fill="${c}"/><rect x="92" y="40" width="13" height="64" rx="4" fill="${c}"/>`,
  previous: (c) => `<path d="M106 40v64L56 72z" fill="${c}"/><rect x="39" y="40" width="13" height="64" rx="4" fill="${c}"/>`,
  seekBack: (c) => `<path d="M48 44a38 38 0 1 1-8 44" ${S(c)}/><path d="M34 30v24h24" ${S(c)}/>`,
  seekForward: (c) => `<path d="M96 44a38 38 0 1 0 8 44" ${S(c)}/><path d="M110 30v24H86" ${S(c)}/>`,
  info: (c) => `${lines(c, 3, 50)}<circle cx="102" cy="96" r="12" fill="${c}"/><path d="M114 96V52l10 6" ${S(c, 7)}/>`,
  shuffle: (c) =>
    `<path d="M28 48h18c18 0 30 48 50 48h18M28 96h18c8 0 14-8 20-18M84 58c4-6 8-10 12-10h18" ${S(c)}/><path d="M104 36l12 12-12 12M104 84l12 12-12 12" ${S(c)}/>`,
  repeat: (c) => `<path d="M34 76V60a12 12 0 0 1 12-12h62M110 68v16a12 12 0 0 1-12 12H36" ${S(c)}/><path d="M98 36l12 12-12 12M46 108 34 96l12-12" ${S(c)}/>`,
  repeatOne: (c) =>
    `<path d="M34 76V60a12 12 0 0 1 12-12h62M110 68v16a12 12 0 0 1-12 12H36" ${S(c)}/><path d="M98 36l12 12-12 12M46 108 34 96l12-12" ${S(c)}/><path d="M68 64l6-4v24" ${S(c, 6)}/>`,
  crossfade: (c) => `<path d="M26 98c26 0 40-52 92-52M26 46c26 0 40 52 92 52" ${S(c)}/>`,
  private: (c) =>
    `<path d="M24 72s18-30 48-30 48 30 48 30-18 30-48 30-48-30-48-30z" ${S(c, 7)}/><circle cx="72" cy="72" r="12" fill="${c}"/><path d="M34 110 110 34" ${S(c, 7)}/>`,
  volume: (c) => `${speaker(c)}<path d="M86 54a26 26 0 0 1 0 36M98 42a42 42 0 0 1 0 60" ${S(c)}/>`,
  volumeUp: (c) => `${speaker(c)}<path d="M98 56v32M82 72h32" ${S(c)}/>`,
  volumeDown: (c) => `${speaker(c)}<path d="M82 72h32" ${S(c)}/>`,
  volumeSet: (c) => `${speaker(c)}<circle cx="98" cy="72" r="14" ${S(c, 7)}/><circle cx="98" cy="72" r="4" fill="${c}"/>`,
  mute: (c) => `${speaker(c)}<path d="M86 58l28 28M114 58 86 86" ${S(c)}/>`,
  heart: (c) => `<path d="${heart}" ${S(c)}/>`,
  heartFilled: (c) => `<path d="${heart}" fill="${c}" ${S(c)}/>`,
  addToPlaylist: (c) => `${lines(c, 3, 52)}<path d="M104 78v32M88 94h32" ${S(c)}/>`,
  removeFromPlaylist: (c) => `${lines(c, 3, 52)}<path d="M88 94h32" ${S(c)}/>`,
  star: (c) => `<path d="${star}" ${S(c, 7)}/>`,
  starFilled: (c) => `<path d="${star}" fill="${c}" ${S(c, 7)}/>`,
  playlist: (c) => `${lines(c, 3, 48)}<path d="M90 76v34l28-17z" fill="${c}"/>`,
  album: (c) => `<circle cx="72" cy="72" r="42" ${S(c)}/><circle cx="72" cy="72" r="10" fill="${c}"/>`,
  artist: (c) => `<circle cx="72" cy="54" r="18" ${S(c)}/><path d="M38 110c4-20 18-30 34-30s30 10 34 30" ${S(c)}/>`,
  mix: (c) => `<path d="M44 34v76M72 34v76M100 34v76" ${S(c, 6)}/><circle cx="44" cy="84" r="10" fill="${c}"/><circle cx="72" cy="56" r="10" fill="${c}"/><circle cx="100" cy="92" r="10" fill="${c}"/>`,
  link: (c) => `<path d="M64 80l16-16M58 62l-8 8a17 17 0 0 0 24 24l8-8M86 82l8-8a17 17 0 0 0-24-24l-8 8" ${S(c)}/>`,
  search: (c) => `<circle cx="64" cy="64" r="28" ${S(c)}/><path d="M86 86l24 24" ${S(c)}/>`,
  radio: (c) => `<circle cx="72" cy="72" r="9" fill="${c}"/><path d="M52 52a28 28 0 0 0 0 40M92 52a28 28 0 0 1 0 40M38 38a48 48 0 0 0 0 68M106 38a48 48 0 0 1 0 68" ${S(c, 7)}/>`,
  daylist: (c) =>
    `<circle cx="72" cy="72" r="20" fill="${c}"/><path d="M72 30v10M72 104v10M30 72h10M104 72h10M42 42l7 7M95 95l7 7M42 102l7-7M95 49l7-7" ${S(c, 7)}/>`,
  discover: (c) => `<circle cx="72" cy="72" r="42" ${S(c)}/><path d="M90 54 80 80 54 90l10-26z" fill="${c}" ${S(c, 4)}/>`,
  liked: (c) => `<rect x="26" y="26" width="92" height="92" rx="18" ${S(c, 7)}/><path d="M72 98 52 78a12 12 0 0 1 20-14 12 12 0 0 1 20 14z" fill="${c}"/>`,
  queue: (c) => `${lines(c, 3, 60)}<path d="M100 84v26l20-13z" fill="${c}"/>`,
  device: (c) => `<rect x="40" y="28" width="64" height="88" rx="10" ${S(c, 7)}/><circle cx="72" cy="84" r="16" ${S(c, 7)}/><circle cx="72" cy="48" r="5" fill="${c}"/>`,
  lyrics: (c) => `<path d="M30 40h84M30 62h60M30 84h72M30 106h40" ${S(c)}/>`,
  miniplayer: (c) => `<rect x="24" y="34" width="96" height="76" rx="10" ${S(c, 7)}/><rect x="70" y="72" width="38" height="26" rx="5" fill="${c}"/>`,
  window: (c) => `<rect x="24" y="34" width="96" height="76" rx="10" ${S(c, 7)}/><path d="M24 54h96" ${S(c, 7)}/>`,
  sleep: (c) => `<path d="M90 30a44 44 0 1 0 26 66A36 36 0 0 1 90 30z" fill="${c}"/>`,
  bookmark: (c) => `<path d="M46 28h52v88L72 96l-26 20z" ${S(c)}/>`,
  request: (c) => `<circle cx="66" cy="66" r="40" ${S(c)}/><circle cx="66" cy="66" r="8" fill="${c}"/><path d="M110 92v32M94 108h32" ${S(c)}/>`,
  listen: (c) =>
    `<path d="M32 92V76a40 40 0 0 1 80 0v16" ${S(c)}/><rect x="26" y="84" width="22" height="34" rx="6" fill="${c}"/><rect x="96" y="84" width="22" height="34" rx="6" fill="${c}"/>`,
  dial: (c) => `<circle cx="72" cy="72" r="40" ${S(c)}/><path d="M72 32v20" ${S(c)}/>`,
};
