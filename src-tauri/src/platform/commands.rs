//! Carrying out what the OS asks for (session buttons, media keys, tray
//! menu) on the player.

use async_trait::async_trait;

use super::media::OsCommand;
use crate::audio::player::PlayerHandle;
use crate::audio::queue::Repeat;
use crate::error::AppResult;

/// The player calls the OS controls need. [`PlayerHandle`] in the app, a
/// recorder in tests.
#[async_trait]
pub trait PlayerControl: Send + Sync {
    async fn toggle(&self) -> AppResult<()>;
    async fn play(&self) -> AppResult<()>;
    async fn pause(&self) -> AppResult<()>;
    async fn next(&self) -> AppResult<()>;
    async fn previous(&self) -> AppResult<()>;
    async fn seek(&self, ms: u64) -> AppResult<()>;
    async fn position(&self) -> AppResult<(u64, u64)>;
    async fn set_shuffle(&self, on: bool) -> AppResult<()>;
    async fn set_repeat(&self, repeat: Repeat) -> AppResult<()>;
}

#[async_trait]
impl PlayerControl for PlayerHandle {
    async fn toggle(&self) -> AppResult<()> {
        PlayerHandle::toggle(self).await
    }
    async fn play(&self) -> AppResult<()> {
        self.resume().await
    }
    async fn pause(&self) -> AppResult<()> {
        PlayerHandle::pause(self).await
    }
    async fn next(&self) -> AppResult<()> {
        PlayerHandle::next(self).await
    }
    async fn previous(&self) -> AppResult<()> {
        PlayerHandle::previous(self).await
    }
    async fn seek(&self, ms: u64) -> AppResult<()> {
        PlayerHandle::seek(self, ms).await
    }
    async fn position(&self) -> AppResult<(u64, u64)> {
        let s = self.state().await?;
        Ok((s.position_ms, s.duration_ms))
    }
    async fn set_shuffle(&self, on: bool) -> AppResult<()> {
        PlayerHandle::set_shuffle(self, on).await
    }
    async fn set_repeat(&self, repeat: Repeat) -> AppResult<()> {
        PlayerHandle::set_repeat(self, repeat).await
    }
}

/// Run one player command. Window commands (`Show`, `Quit`) are handled by
/// the caller and ignored here. Returns whether it was a player command.
pub async fn run(player: &dyn PlayerControl, cmd: OsCommand) -> AppResult<bool> {
    // The phone's lock screen, Control Centre and the car all come here.
    #[cfg(target_os = "ios")]
    tracing::info!(?cmd, "system command");
    match cmd {
        OsCommand::Toggle => player.toggle().await?,
        OsCommand::Play => player.play().await?,
        OsCommand::Pause => player.pause().await?,
        OsCommand::Next => player.next().await?,
        OsCommand::Previous => player.previous().await?,
        OsCommand::SeekTo(ms) => player.seek(ms).await?,
        OsCommand::SeekBy(delta) => {
            let (pos, dur) = player.position().await?;
            let to = pos.saturating_add_signed(delta);
            player.seek(if dur > 0 { to.min(dur) } else { to }).await?
        }
        OsCommand::Shuffle(on) => player.set_shuffle(on).await?,
        OsCommand::Repeat(r) => player.set_repeat(r).await?,
        OsCommand::Show | OsCommand::Quit => return Ok(false),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    #[derive(Default)]
    struct Rec(Mutex<Vec<String>>);

    #[async_trait]
    impl PlayerControl for Rec {
        async fn toggle(&self) -> AppResult<()> {
            self.0.lock().push("toggle".into());
            Ok(())
        }
        async fn play(&self) -> AppResult<()> {
            self.0.lock().push("play".into());
            Ok(())
        }
        async fn pause(&self) -> AppResult<()> {
            self.0.lock().push("pause".into());
            Ok(())
        }
        async fn next(&self) -> AppResult<()> {
            self.0.lock().push("next".into());
            Ok(())
        }
        async fn previous(&self) -> AppResult<()> {
            self.0.lock().push("previous".into());
            Ok(())
        }
        async fn seek(&self, ms: u64) -> AppResult<()> {
            self.0.lock().push(format!("seek {ms}"));
            Ok(())
        }
        async fn position(&self) -> AppResult<(u64, u64)> {
            Ok((5_000, 12_000))
        }
        async fn set_shuffle(&self, on: bool) -> AppResult<()> {
            self.0.lock().push(format!("shuffle {on}"));
            Ok(())
        }
        async fn set_repeat(&self, repeat: Repeat) -> AppResult<()> {
            self.0.lock().push(format!("repeat {repeat:?}"));
            Ok(())
        }
    }

    #[tokio::test]
    async fn os_commands_map_to_player_commands() {
        let p = Rec::default();
        for c in [
            OsCommand::Toggle,
            OsCommand::Play,
            OsCommand::Pause,
            OsCommand::Next,
            OsCommand::Previous,
            OsCommand::SeekTo(42_000),
            OsCommand::SeekBy(10_000),
            OsCommand::SeekBy(-10_000),
            OsCommand::Shuffle(true),
            OsCommand::Repeat(Repeat::One),
        ] {
            assert!(run(&p, c).await.unwrap());
        }
        assert!(!run(&p, OsCommand::Show).await.unwrap());
        assert_eq!(
            *p.0.lock(),
            ["toggle", "play", "pause", "next", "previous", "seek 42000", "seek 12000", "seek 0", "shuffle true", "repeat One"]
        );
    }
}
