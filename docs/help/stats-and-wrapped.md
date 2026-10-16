# Stats and recaps

What the Stats page shows, how monthly recaps and comparing with a friend work, and how to bring in plays from before Platter.

![The Stats page for the last 4 weeks](images/stats.png)

## Opening Stats

Open Stats from the app menu or the Stats row in Your Library. Everything is counted on this computer, from what you've played in Platter and any history you brought in. Skips don't count. To hide the page, turn off Settings, Library, Stats, Stats page.

## Picking a range

The chips at the top pick the days to count: 4 weeks, 6 months, This year, A year, A month, All time or Custom. A year and A month add a picker, and Custom lets you choose two dates.

The top of the page sums up the range: minutes listened, plays, songs, artists, albums and days, your top artist, your top song, your longest streak and how many new artists you found.

## Top lists

Your top songs, artists, albums and genres come next, ranked by plays, then minutes. Show more lists the rest. Play these plays the list, and Save as playlist keeps it. Unreleased songs carry an Unreleased mark.

## Charts

When you listened tells three short stories, each with a small chart:

- Listening days, as month calendars.
- Minutes per day, or per month for long ranges, with the best one marked.
- Your day, as 24 hourly bars beside the days of the week.

Arrow keys move along a chart, and the line under it gives the numbers.

## Moments

Moments are small stories from the range: your first song, On repeat (the day you played one song over and over), Biggest discovery (the new artist you played most) and Welcome back (a song you came back to after at least 30 days). Choose a card to play the song or open the artist.

## Monthly recaps

With Settings, Library, Stats, Show monthly stats on, each past month gets a card with its minutes, the change on the month before, and its top artist and song. Choose Share to make an image in Story or Square shape, then Copy image or Save image. Unreleased songs are left off the card and nothing is uploaded.

## A whole year

To look back on a year, pick This year or A year. The page then shows the whole year, and Moments start with your first song of that year.

<!-- TODO: no separate yearly recap or Wrapped-style story was found in the code (only the This year and A year ranges and monthly recaps). Describe it here if one is added. -->

## Stats on other pages

Settings, Library, Stats, Stats strip on song, artist and album pages shows your plays, minutes, first and last listen and all-time rank under an artist's or album's buttons. Songs get Your stats in their menu.

## Compare with a friend

Turn on Settings, Library, Stats, Compare stats with friends (it starts off). When a Platter friend shares their stats with you, the Stats page shows your taste overlap, artists in common, and their songs you haven't played. Only a summary is shared: up to 50 top artists and songs from about the last six months, never your history. Unreleased songs stay out unless you both agree.

<!-- TODO: how a friend's summary reaches this computer is unclear; the code stores and compares summaries but nothing in the app sends or receives them yet. -->

## Plays from before Platter

Stats, mixes, On Repeat and recaps can also count plays from elsewhere. Each source has an Include switch under Settings, Library, Stats, and a play reported by more than one source counts once.

- Include my Plex listening history: what you played on your Plex server before Platter and in other Plex apps. Only your own plays count, never other people's on the server.
- Include Tautulli history: shows once you set Settings, Integrations, Tautulli, Tautulli address and Tautulli key. The key is kept in this computer's keychain. Plays too short to count are left out.
- Include Last.fm scrobbles: shows once you connect Last.fm under Settings, Integrations, Scrobbling. Scrobbles are matched to your library by artist, song and album. Songs you don't have still count for their artists.

The first read brings in the whole history in the background; new plays follow every 6 hours. Remove imported plays from a source deletes only what came from there; your own plays in Platter stay.

History files from other apps go through [Library import](library-import.md).
