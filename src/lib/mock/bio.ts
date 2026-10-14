/** Made-up artist biographies for the mock backend. */
const OPENINGS = [
  "started as a weekend project in a rented rehearsal room and never quite stopped",
  "grew out of late-night sessions on borrowed gear",
  "came together after a run of house shows that got out of hand",
  "began as one person with a four-track and a lot of patience",
];
const MIDDLES = [
  "Early releases were rough and loud, recorded in single takes with whatever microphones were around.",
  "The sound leans on warm tape hiss, patient arrangements and choruses that arrive late.",
  "Records tend to start sparse and build, with long instrumental passages between the songs.",
  "Live sets are known for reworking old songs until they are barely recognisable.",
];
const ENDINGS = [
  "Most of the catalogue is self-released, alongside a steady trickle of demos and alternate takes.",
  "A few unreleased sessions still circulate, and some of them are better than the official versions.",
  "The band keeps a slow pace: a record every couple of years, singles in between.",
];

export function mockBio(name: string, i: number): string {
  const long = i % 2 === 0;
  const parts = [
    `${name} ${OPENINGS[i % OPENINGS.length]}.`,
    MIDDLES[i % MIDDLES.length],
    ENDINGS[i % ENDINGS.length],
  ];
  if (long) parts.push(MIDDLES[(i + 1) % MIDDLES.length], ENDINGS[(i + 1) % ENDINGS.length]);
  return parts.join(" ");
}
