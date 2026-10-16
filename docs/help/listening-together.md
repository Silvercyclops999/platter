# Listening together

How to host a listening session, invite people, share your music through Plex, see what friends play, show sessions on Discord and share links to music.

![A listening session strip with the People list open](images/listening-session.png)

## Start a listening session

1. Choose Start a listening session from a song, album or playlist menu, or from the Listen along card in the queue panel.
2. A strip above the player bar shows the session and who is listening.
3. Choose Invite on the strip to ask people in.

## Invite links and QR codes

The invite shows a link and a QR code. Choose Copy link and send it, or let someone scan the code. When they open the link, Platter opens and joins the session.

Each link works for one person, once. It stops working after the time in Settings, Social, Listening sessions, Invites expire after (1 hour, 24 hours or 7 days). Choose New link for the next person.

<!-- TODO: confirm how a guest uses the QR code, since Platter is a desktop app and the code holds the same platter:// join link. -->

## What guests hear and do

Each guest plays from your Plex library when they have access to it, else from their own copy of the same song. A copy of a different length shows as Another version. When a guest has no copy, their player pauses with Not in your library and picks up at the next song they have.

- Guests can always add songs with Add to the session's queue, and the queue shows who added each one.
- With Settings, Social, Listening sessions, Let others change what's playing on, guests can also play, pause, skip and reorder. Off, they can only add.
- Your name in listening sessions is what others see. Leave it empty to use your Plex name. Nobody sees your email, location or computer.
- Unreleased songs only play for people with access to your library. Turn on Hide unreleased titles from guests to hide their title and artist too.

## Let them listen from my server

When a guest can't hear a song because it isn't on any server they can reach, a card above the strip says so: Robin can't hear this. Choose Let them listen from my server to invite them to your Plex server.

- The first time, pick which music libraries to share. They become your sharing defaults, so after that it's one click. Your other libraries (films, shows, photos) are never shared.
- Their Platter tells yours their Plex username, sealed, only when you ask and only if Accept invites inside Platter is on for them. Plex sends the invite; Platter can't share any other way.
- They accept in their Platter, your server joins theirs, and they hear your copy of the song from then on.

Turn the card off with Settings, Social, Listening sessions, Offer "Let them listen from my server". If their Platter doesn't take invites, share from Settings, Sharing with their Plex username instead.

## Sharing your music

Settings, Sharing, Share my music invites anyone to your Plex server, in or out of a session:

1. Type their Plex username or email, or choose Make a link instead for a link and QR code. Their Platter opens the link and asks your Platter for an invite through the relay. A link works for one person, once, and you need Platter open while they use it.
2. Pick the music libraries to share. Each one says how many unreleased albums it holds.
3. Allow downloads is off unless you turn it on (Allow downloads for people I share with sets the default).
4. Choose Share. Plex may email them about it too.

Only the server's owner can share it. Plex's own rules still decide whether friends can play your music away from your home network.

## Keeping unreleased music home

Before you share, Platter counts the unreleased albums in what you picked, using the same release status as everywhere else ([how Platter decides](archive-and-release-status.md#how-platter-decides)). With Hide my unreleased music from people I share with on (it starts on), Platter labels those albums in Plex and the share leaves that label out, so friends never see them.

Leaving labelled albums out is one of Plex's sharing restrictions, which need Plex Pass on your account. Without it, Platter shares nothing until you choose: share the libraries that hold nothing unreleased, or cancel. A library Platter hasn't read yet can't be checked, so include it in Settings, Library first.

## People with access

Settings, Sharing, People with access lists everyone your server is shared with: the libraries they see, when they were last on your server, whether unreleased music is hidden from them and whether they can download. Choose Remove to take their access away. They stay your Plex friend.

## Invites to you

When a friend shares their server with you, Platter shows a notification and a card, in Listen along too while you listen with them. Choose Accept: their server and its music libraries join your library, and Platter finds the best way to reach it. You don't need to visit plex.tv. Turn this off with Settings, Sharing, Accept invites inside Platter; invites then wait in Plex as usual.

## Removing people and ending

Choose People on the strip to see everyone. Next to each guest is a Remove button. A removed guest leaves the session at once.

As host, End session stops it for everyone and forgets every invite. As a guest, choose Leave.

## The relay

Everyone connects only to a relay, never to each other, so nobody learns anyone else's address, and messages are sealed end to end. The default relay is ntfy.sh (free, about 250 messages a day per connection). To use your own ntfy server, put its address in Settings, Social, Listening sessions, Relay for listening sessions.

## Discord

While you host, Settings, Integrations, Discord, Show the Listen along button on Discord makes your Discord status show the session and how many are listening. Joining from Discord isn't there yet, so send the invite link. How unreleased songs show on Discord is on [Archive and release status](archive-and-release-status.md#how-unreleased-music-is-treated).

## Friend activity

The people button in the top bar shows what others on your Plex server played in the last week. Turn it on or off with Settings, Social, Friend activity, Show friend activity. Only the server's owner can see it, because Plex shares listening with the owner only.

Share my activity with Platter friends lets Plex show what you're playing right now. It's off until you turn it on. While it's off, or in a private session, friends don't see you live. Songs you finish still go into Plex's history, which the server's owner can see.

## Share links

The Share menu on songs, albums, artists and playlists offers:

- Copy song link (or album link): a song.link page that opens the music in any music service. It needs Settings, Sharing, Public links for released music. Platter finds it with iTunes Search, sending only the artist, title and album. Unreleased music never gets one.
- Copy Platter link: opens the item in Platter for people on your Plex server. It never holds your server's address.
- Copy link at a time, for the song playing now.
- Share card (not for artists): an image to copy or save, with a QR code (Share cards include a QR code) and the song.link line (Include the song.link line). Nothing is uploaded.
