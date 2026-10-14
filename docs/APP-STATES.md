# App states

What Platter does when something goes wrong, where each case is detected,
where the user sees it, and the automated test that fakes it. The rule: say
what's wrong in plain words and keep playing whatever still can.

The core owns the decision. `PlexState` keeps a health state machine
(`src-tauri/src/plex/health.rs`) that turns pings, server events and the
watchers in `src-tauri/src/monitor/` into a `Health` snapshot with one banner
(kind + message). The UI shows that banner at the top of the main view
(`src/shell/HealthBanner.tsx`), the status dot in the sidebar, and toasts.
While `Health.cachedOnly` is true, track rows that aren't stored on this
computer are dimmed (`cached_tracks` command).

Rust tests run with `cargo test` in `src-tauri`; UI tests with `npm test`.
To see a banner in a browser, run `npx vite` and open `/?health=<state>`
(states are listed in `src/lib/mock/health.ts`). Screenshots of each banner
are in `docs/screenshots/app-states/`.

## Server and connection

| Situation | Detected in | User sees | Tests | Status |
| --- | --- | --- | --- | --- |
| Server offline or unreachable | 30 s ping in `plex/state.rs`; a failed ping re-races every address from plex.tv before calling it offline | Banner "Your server is offline. Last seen 4 min ago." (age kept current by the UI), red dot. Uncached tracks dimmed, cached ones play. Toast "Your server is back." when it returns | `health::offline_after_failed_rediscover_with_last_seen`, `state::server_down_with_internet_says_server_offline`, `state::server_back_after_offline_is_announced`, `state::offline_server_then_health_tick`, `engine_tests::plays_from_disk_cache_when_server_is_gone`; UI `appstates` offline banner, dimming, back toast | Done |
| Server starting, restarting or updating | HTTP 503 on ping, server shutdown and update notifications | "Server is starting…" or "Server is updating…"; after 2 minutes "Your server is taking too long to come back. Check that Plex is running." | `health::starting_waits_then_errors_after_two_minutes`, `health::updating_shows_updating`, `health::shutdown_notice_opens_restart_window`; UI `appstates` starting/updating/startTimeout | Done |
| Only remote reachable | Connection race picks remote | "Remote" badge by the server name, blue dot. Plays the original file | `health::remote_only_shows_remote_without_banner`, `bridge::relay_drops_quality_local_and_remote_keep_original`; UI `appstates` status dot | Done (away-from-home quality is the original file until a quality setting exists) |
| Only relay reachable | Connection race picks relay | "Relay" badge plus a tip banner. Files above 320 kbps (or of unknown bitrate) are transcoded to 320 kbps MP3 | `health::relay_only_shows_relay_tip`, `bridge::relay_drops_quality_local_and_remote_keep_original`; UI `appstates` relay | Done |
| Server address changed | Ping fails, re-race with fresh addresses from plex.tv | Nothing | `health::address_changed_rediscovers_silently`, `state::health_tick_rediscovers_new_address` | Done |
| plex.tv down | Sign-in restore falls back to the cached server list | Nothing if the server answers | `state::restore_with_plex_tv_down_uses_cached_server`, `health::plex_tv_down_with_cached_server_is_silent` | Done |
| Computer offline or Wi-Fi switch | `monitor/net.rs` checks the default route every 5 s (no packets sent); a failed re-race where plex.tv is also unreachable | "You're offline." at once when the route disappears. A route change reconnects right away. Pre-cached music keeps playing | `net::wifi_switch_and_cable_pull`, `net::starting_without_network_reports_lost`, `health::no_internet_says_you_are_offline`, `health::no_network_route_is_offline_at_once`, `state::computer_offline_says_you_are_offline`, `state::network_lost_is_immediate_and_reconnect_recovers` | Done |
| Stream drops mid-song | Audio engine HTTP reader | At most a brief pause; resumes from the last byte | `engine_tests::dropped_connection_resumes_without_changing_audio`, `engine_tests::http_source_resumes_byte_exact` | Done |

## Account

| Situation | Detected in | User sees | Tests | Status |
| --- | --- | --- | --- | --- |
| Token expired or revoked | 401/403 on the ping or from plex.tv | The app stays open with banner "Please sign in again." and a Sign in button (the welcome screen opens over the app, with Not now). Music on this computer keeps playing | `health::token_revoked_requires_sign_in`, `health::token_rejected_keeps_app_open_with_sign_in_banner`, `state::revoked_token_on_restore_signs_out`, `state::health_tick_detects_revoked_token`; UI `appstates` token expired | Done |
| Server access removed | plex.tv no longer lists the selected server | "You no longer have access to this server." With other servers, the server picker opens with that message (with exactly one other, a toast and a switch to it). With none, a banner with "Use another account" | `health::access_removed_banner`, `state::access_removed_when_server_disappears`; UI `appstates` accessRemoved | Done |
| No Plex Pass or sonic analysis off | Not yet: no current feature needs either | Each feature that needs them will explain it | None yet | Not applicable until sonic features (radio, mixes) are built |

## Library and files

| Situation | Detected in | User sees | Tests | Status |
| --- | --- | --- | --- | --- |
| Server up, music files unavailable (array stopped, drive unmounted) | `monitor/files.rs`: every minute while connected, 3 random tracks are checked with `/library/metadata/{id}?checkFiles=1` (a metadata call, so it never cuts a playing stream). Two file failures in a row during playback trigger a check right away. Unavailable only when every checked file is unreadable | "Your server is up, but your music files aren't available." Uncached tracks dimmed; cached ones play. Rechecked every minute; the banner clears by itself | `files::array_stopped_means_unavailable`, `files::one_missing_file_is_not_an_outage`, `files::no_answers_keep_the_last_verdict`, `files::failure_watch_counts_only_new_file_failures`, `client::file_readable_reads_check_files_flags`, `health::files_unavailable_is_cached_only_and_recovers`, `state::files_unavailable_and_back`; UI `appstates` filesUnavailable | Done, needs a real-server check (below) |
| One file missing or corrupt | Audio engine open/decode | Toast "Skipped: file unavailable"; skipped and logged to the Problems list | `player_tests::missing_file_is_skipped_with_error_and_problem_logged`, `player_tests::missing_first_track_skips_to_the_next`; UI `appstates` skipped toast | Done |
| Unsupported format | Decoder fails on the original | Nothing; Plex transcodes it | `engine_tests::undecodable_original_falls_back_to_transcode` | Done |
| Transcoder failing | Transcode request fails | Nothing unless it repeats: the track is skipped with the toast above | `engine_tests::transcode_also_failing_reports_failure` | Done (the original is already tried first) |
| Library scanning | Websocket activity (scan running) and timeline entries (track created, then processed) | "Adding new music… 120 of 340" (only when the scan finds new tracks). New albums show up as they land via the coalesced re-sync | `health::scan_counts_new_tracks`, `health::scan_progress_clears_on_disconnect`, `state::scan_progress_from_server_events`; UI `appstates` scanning | Done |
| No music library | Library sync finds no music section | "No music library found." with a How to add one guide. Re-synced every minute until one appears | `db::tests::no_music_library_is_reported`, `library::no_music_library_sets_and_clears_the_flag`, `health::no_music_library_banner`; UI `appstates` noMusicLibrary and guide | Done |
| Library replaced | Sync sees the library's uuid change; cache wiped and rebuilt | Toast "This library changed." | `db::tests::library_replaced_wipes_and_rebuilds`, `library::library_replaced_tells_the_user`; UI `appstates` toast | Done |

## This device

| Situation | Detected in | User sees | Tests | Status |
| --- | --- | --- | --- | --- |
| Headphones unplugged or Bluetooth drop | Audio output reports the device is gone | Playback pauses; Play resumes on the new default output | `player_tests::device_loss_pauses_and_resume_reopens`; UI `appstates` headphones | Done |
| Disk full | `monitor/disk.rs` checks free space every minute | Below 512 MB free: trims the oldest pre-cached audio, then shrinks the artwork cache. If still low, "Not enough space." The artwork cap comes back once space recovers | `disk::plan_trims_only_when_low`, `disk::disk_full_trims_oldest_audio_first_and_keeps_recent`, `health::low_disk_banner_survives_server_switch`; UI `appstates` diskFull | Done (downloads come later; pre-cache files are capped and trimmed instead) |
| Sleep and wake | `monitor/net.rs`: a wall-clock jump much longer than the 5 s tick | Nothing. Reopens the notification socket and pings at once. The queue lives in the core, so it's unchanged | `net::wake_from_sleep_is_seen_from_the_clock_jump`, `net::wake_on_another_network`, `state::network_lost_is_immediate_and_reconnect_recovers` | Done |
| Update available | `updates.rs` checks GitHub releases and downloads in the background | Toast "Platter 0.2.0 is ready. Restart to install it." with Restart. A failed download changes nothing | UI `appstates` update toast | Done (the installer keeps the old version if it fails) |
| Crash | Panic hook in `monitor/crash.rs` | Nothing on relaunch: the queue and position come back from `queue.json`, which is written a moment after every change. A crash report is saved locally in `crash-reports/` and never sent | `player_tests::crash_without_flush_restores_queue_and_position`, `player_tests::queue_and_position_survive_a_restart`, `crash::writes_report_and_keeps_the_last_few` | Done |

## Checking by hand against a real server

Each of these is covered by a test that fakes it, and should also be tried
once for real:

1. Server offline: stop Plex Media Server. Within 30 s the banner says the
   server is offline; cached albums play, others are dimmed. Start Plex: a
   "Your server is back." toast.
2. Files unavailable: stop the disk array (or unmount the music drive) while
   Plex keeps running. Within a minute the files banner shows. Check what
   `/library/metadata/{id}?checkFiles=1` returns for a track (the `exists`
   and `accessible` attributes on `Part`) and note it in docs/PLEX-API.md.
   Start the array: the banner clears within a minute.
3. Computer offline: pull the network cable (Wi-Fi off). "You're offline."
   should appear within about 5 s and pre-cached music should keep playing.
   Plug back in: it reconnects without waiting for the next ping.
4. Sleep: put the computer to sleep for a few minutes while connected, wake
   it, and check the status dot is green again within a few seconds.
5. Token revoked: sign the device out at plex.tv (Settings, Authorized
   Devices). On the next ping the app shows "Please sign in again." and keeps
   playing cached music.
6. Scan: add a few albums to the music folder and scan the library in Plex.
   "Adding new music… N of M" shows while it runs and new albums appear.
