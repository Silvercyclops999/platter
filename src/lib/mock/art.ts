/**
 * Deterministic generated artwork for the mock backend: SVG gradients as data
 * URLs. Nothing is copied from anywhere; the shapes are derived from a hash of
 * the Plex image path so the same item always gets the same "cover".
 */
import { hashString, mulberry32 } from "./fixtures";

const cache = new Map<string, string>();

export function mockArtUrl(path: string): string {
  const hit = cache.get(path);
  if (hit) return hit;
  const r = mulberry32(hashString(path));
  const h1 = Math.floor(r() * 360);
  const h2 = (h1 + 30 + Math.floor(r() * 120)) % 360;
  const sat = 45 + Math.floor(r() * 35);
  const angle = Math.floor(r() * 360);
  const isArtist = /\/library\/metadata\/1\d\d\//.test(path);
  const isPlaylist = path.startsWith("/playlists/");
  const motif = Math.floor(r() * 4);

  let shapes = "";
  if (isArtist) {
    // abstract "portrait": head + shoulders silhouette
    shapes = `<circle cx="150" cy="122" r="58" fill="hsl(${h2} ${sat}% 18% / .55)"/>
<ellipse cx="150" cy="300" rx="118" ry="96" fill="hsl(${h2} ${sat}% 18% / .55)"/>`;
  } else if (isPlaylist) {
    shapes = [0, 1, 2, 3]
      .map(
        (i) =>
          `<rect x="${(i % 2) * 150}" y="${Math.floor(i / 2) * 150}" width="150" height="150" fill="hsl(${(h1 + i * 40) % 360} ${sat}% ${30 + i * 8}% / .55)"/>`,
      )
      .join("");
  } else if (motif === 0) {
    shapes = `<circle cx="150" cy="150" r="96" fill="none" stroke="hsl(${h2} 90% 85% / .35)" stroke-width="2"/>
<circle cx="150" cy="150" r="64" fill="none" stroke="hsl(${h2} 90% 85% / .25)" stroke-width="2"/>
<circle cx="150" cy="150" r="14" fill="hsl(${h2} 90% 85% / .6)"/>`;
  } else if (motif === 1) {
    const y = 90 + Math.floor(r() * 120);
    shapes = `<rect x="0" y="${y}" width="300" height="${20 + Math.floor(r() * 40)}" fill="hsl(${h2} 85% 70% / .35)"/>
<circle cx="${60 + Math.floor(r() * 180)}" cy="${y - 20}" r="${30 + Math.floor(r() * 30)}" fill="hsl(${h1} 85% 80% / .45)"/>`;
  } else if (motif === 2) {
    shapes = Array.from({ length: 6 }, (_, i) => {
      const yy = 40 + i * 40;
      return `<path d="M0 ${yy} Q75 ${yy - 30} 150 ${yy} T300 ${yy}" fill="none" stroke="hsl(${h2} 80% 80% / ${0.15 + i * 0.06})" stroke-width="3"/>`;
    }).join("");
  } else {
    shapes = `<polygon points="150,46 254,236 46,236" fill="none" stroke="hsl(${h2} 90% 85% / .45)" stroke-width="3"/>
<circle cx="150" cy="170" r="30" fill="hsl(${h1} 90% 80% / .35)"/>`;
  }

  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 300 300" width="300" height="300">
<defs><linearGradient id="g" gradientTransform="rotate(${angle} .5 .5)">
<stop offset="0" stop-color="hsl(${h1} ${sat}% 42%)"/><stop offset="1" stop-color="hsl(${h2} ${sat}% 22%)"/>
</linearGradient></defs>
<rect width="300" height="300" fill="url(#g)"/>${shapes}</svg>`;
  const url = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
  cache.set(path, url);
  return url;
}
