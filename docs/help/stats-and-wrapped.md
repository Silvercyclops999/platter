# Stats and recaps

What your profile and the Stats page show, how monthly recaps and comparing with a friend work, and how to bring in plays from before Platter.

![The Stats page for the last 4 weeks](images/stats.png)

## Your profile

Choose Profile at the top of the app menu (your picture in the top bar). It shows your Plex picture and name, how many playlists you have and how many plays this month, then this month's minutes, plays and longest streak. Under that come your top artists and top tracks this month, your playlists and your friends, with links to all your stats and to Wrapped. Show all opens Stats on this month, and Compare on a friend opens the Compare tab with them.

Your top artists and tracks are only visible to you. Your playlists and friends are meant for the people you share your music with. Nothing on the profile is public or sent anywhere: it's drawn on this computer.

## Opening Stats

Open Stats from the app menu, the Stats row in Your Library, or Your profile. Everything is counted on this computer, from what you've played in Platter and any history you brought in. Skips don't count. To hide the page, turn off Settings, Library, Show the Stats page.

## Picking a range

The tabs at the top are Overview, Recaps, Wrapped, Diary and Compare. Wrapped shows while Wrapped is on and Diary while you keep a listening diary. Arrow keys move between them.

Under the tabs, the range picks the days to count: 4 weeks, 6 months, Year, Month, All time or Custom. Year and Month add a picker (Year starts on this year), and Custom lets you choose two dates.

Overview starts by summing up the range: minutes listened, plays, songs, artists, albums and days, the share your top artist took, your longest streak and how many new artists you found. Your top song and artist lead the top lists just below.

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

Recaps shows a card for each month in the range (up to 12, newest first). Wrapped lists your Wrapped for each year, this year's included, and your Top Songs lists. With Monthly recaps ticked under Settings, Library, Show the Stats page, each month gets a card with its minutes, the change on the month before, and its top artist and song. Choose Share to make an image in Story or Square shape, then Copy image or Save image. Unreleased songs are left off the card and nothing is uploaded.

## A whole year

To look back on a year, pick Year, then this year or an older one. The page then shows the whole year, and Moments start with your first song of that year.

<!-- TODO: no separate yearly recap or Wrapped-style story was found in the code (only the Year range and monthly recaps). Describe it here if one is added. -->

## Stats on other pages

A stats strip on song, artist and album pages (ticked under Settings, Library, Show the Stats page) shows your plays, minutes, first and last listen and all-time rank under an artist's or album's buttons. Songs get Your stats in their menu.

## Compare with a friend

Tick Compare stats with friends under Settings, Library, Show the Stats page (it starts off). When a Platter friend shares their stats with you, the Compare tab shows your taste overlap, artists in common, and their songs you haven't played. Only a summary is shared: up to 50 top artists and songs from about the last six months, never your history. Unreleased songs stay out unless you both agree.

<!-- TODO: how a friend's summary reaches this computer is unclear; the code stores and compares summaries but nothing in the app sends or receives them yet. -->

## Plays from before Platter

Stats, mixes, On Repeat and recaps can also count plays from elsewhere. Each source has an Include switch, and a play reported by more than one source counts once.

- Settings, Library, Include my Plex listening history: what you played on your Plex server before Platter and in other Plex apps. Only your own plays count, never other people's on the server.
- Include Tautulli history, on Settings, Integrations, Tautulli: shows once you set Tautulli address and Tautulli key. The key is kept in this computer's keychain. Plays too short to count are left out.
- Include Last.fm scrobbles, on Settings, Integrations, Last.fm and ListenBrainz: shows once you connect Last.fm there. Scrobbles are matched to your library by artist, song and album. Songs you don't have still count for their artists.

The first read brings in the whole history in the background; new plays follow every 6 hours. Remove imported plays from a source deletes only what came from there; your own plays in Platter stay.

History files from other apps go through [Library import](library-import.md).
