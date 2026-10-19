//! The iPhone's now-playing session: the lock screen, Control Centre, the
//! Dynamic Island and headphone buttons. Track details go to
//! `MPNowPlayingInfoCenter`; the buttons come back through
//! `MPRemoteCommandCenter` and reach the player as [`OsCommand`]s.

use std::ptr::NonNull;
use std::sync::Arc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{ClassType, msg_send};
use objc2_foundation::NSObjectProtocol;
use objc2_foundation::{NSDictionary, NSNumber, NSString};
use objc2_media_player::{
    MPChangePlaybackPositionCommandEvent, MPMediaItemPropertyAlbumTitle, MPMediaItemPropertyArtist,
    MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration, MPMediaItemPropertyTitle,
    MPNowPlayingInfoCenter, MPNowPlayingInfoPropertyElapsedPlaybackTime, MPNowPlayingInfoPropertyPlaybackRate,
    MPRemoteCommand, MPRemoteCommandCenter, MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
};

use super::media::{CommandFn, MediaSession, NowPlaying, OsCommand, Playback};

// The app's Xcode project links frameworks for Swift and Objective-C code
// only; ask the linker for MediaPlayer the way a compiled module would.
std::arch::global_asm!(".linker_option \"-framework\", \"MediaPlayer\"");

pub struct IosSession {
    /// The command targets, removed again when the session goes.
    targets: Vec<(Retained<MPRemoteCommand>, Retained<AnyObject>)>,
    now: NowPlaying,
    playback: Playback,
    artwork: Option<Retained<AnyObject>>,
}

// SAFETY: MPNowPlayingInfoCenter is safe to use from any thread. The command
// targets are only added (in `create`, on the main thread) and removed (on
// drop); MPRemoteCommand guards its target list itself.
unsafe impl Send for IosSession {}

impl IosSession {
    pub fn create(cb: CommandFn) -> Result<Self, String> {
        let cb: Arc<CommandFn> = Arc::new(cb);
        let mut targets = Vec::new();
        // SAFETY: plain calls on the shared command center; each handler only
        // reads the event it is given and returns a status.
        unsafe {
            let center = MPRemoteCommandCenter::sharedCommandCenter();
            let simple: [(Retained<MPRemoteCommand>, OsCommand); 5] = [
                (center.playCommand(), OsCommand::Play),
                (center.pauseCommand(), OsCommand::Pause),
                (center.togglePlayPauseCommand(), OsCommand::Toggle),
                (center.nextTrackCommand(), OsCommand::Next),
                (center.previousTrackCommand(), OsCommand::Previous),
            ];
            for (command, os) in simple {
                let cb = cb.clone();
                let block = RcBlock::new(move |_e: NonNull<MPRemoteCommandEvent>| {
                    cb(os);
                    MPRemoteCommandHandlerStatus::Success
                });
                command.setEnabled(true);
                let target = command.addTargetWithHandler(&block);
                targets.push((command, target));
            }
            // Dragging the lock screen's progress bar.
            let seek: Retained<MPRemoteCommand> = Retained::into_super(center.changePlaybackPositionCommand());
            let cb = cb.clone();
            let block = RcBlock::new(move |e: NonNull<MPRemoteCommandEvent>| {
                let event = e.as_ref();
                if !event.isKindOfClass(MPChangePlaybackPositionCommandEvent::class()) {
                    return MPRemoteCommandHandlerStatus::CommandFailed;
                }
                let event = &*(event as *const MPRemoteCommandEvent as *const MPChangePlaybackPositionCommandEvent);
                let secs = event.positionTime().max(0.0);
                cb(OsCommand::SeekTo((secs * 1000.0) as u64));
                MPRemoteCommandHandlerStatus::Success
            });
            seek.setEnabled(true);
            let target = seek.addTargetWithHandler(&block);
            targets.push((seek, target));
        }
        Ok(Self {
            targets,
            now: NowPlaying::default(),
            playback: Playback::Stopped,
            artwork: None,
        })
    }

    /// Sends the whole now-playing dictionary; the OS has no partial update.
    fn publish(&self) {
        let (position, rate) = match self.playback {
            Playback::Stopped => {
                // SAFETY: clearing the info is always allowed.
                unsafe { MPNowPlayingInfoCenter::defaultCenter().setNowPlayingInfo(None) };
                return;
            }
            Playback::Paused(p) => (p, 0.0),
            Playback::Playing(p) => (p, 1.0),
        };
        // SAFETY: the keys are the framework's own constants and every value
        // is an NSString, NSNumber or MPMediaItemArtwork, as the API expects.
        unsafe {
            let mut keys: Vec<&NSString> = vec![
                MPMediaItemPropertyTitle,
                MPMediaItemPropertyArtist,
                MPMediaItemPropertyAlbumTitle,
                MPMediaItemPropertyPlaybackDuration,
                MPNowPlayingInfoPropertyElapsedPlaybackTime,
                MPNowPlayingInfoPropertyPlaybackRate,
            ];
            let title = NSString::from_str(&self.now.title);
            let artist = NSString::from_str(&self.now.artist);
            let album = NSString::from_str(&self.now.album);
            let duration = NSNumber::new_f64(self.now.duration_ms as f64 / 1000.0);
            let elapsed = NSNumber::new_f64(position as f64 / 1000.0);
            let rate = NSNumber::new_f64(rate);
            let mut values: Vec<&AnyObject> = vec![&title, &artist, &album, &duration, &elapsed, &rate];
            if let Some(art) = &self.artwork {
                keys.push(MPMediaItemPropertyArtwork);
                values.push(art);
            }
            let info = NSDictionary::from_slices(&keys, &values);
            MPNowPlayingInfoCenter::defaultCenter().setNowPlayingInfo(Some(&info));
        }
    }
}

/// The cover as an `MPMediaItemArtwork`, from a file in the art cache.
fn artwork(path: &std::path::Path) -> Option<Retained<AnyObject>> {
    let path = path.to_str()?;
    // SAFETY: UIImage and MPMediaItemArtwork are looked up by name and sent
    // their documented initialisers; a missing class or unreadable file
    // gives None.
    unsafe {
        let image_class = AnyClass::get(c"UIImage")?;
        let image: Option<Retained<AnyObject>> =
            msg_send![image_class, imageWithContentsOfFile: &*NSString::from_str(path)];
        let image = image?;
        let art_class = AnyClass::get(c"MPMediaItemArtwork")?;
        let art: Option<Retained<AnyObject>> = msg_send![msg_send![art_class, alloc], initWithImage: &*image];
        art
    }
}

impl MediaSession for IosSession {
    fn set_metadata(&mut self, now: &NowPlaying) -> Result<(), String> {
        if self.now.cover != now.cover {
            self.artwork = now.cover.as_deref().and_then(artwork);
        }
        self.now = now.clone();
        self.publish();
        Ok(())
    }

    fn set_playback(&mut self, playback: Playback) -> Result<(), String> {
        self.playback = playback;
        self.publish();
        Ok(())
    }
}

impl Drop for IosSession {
    fn drop(&mut self) {
        // SAFETY: removing targets this session added, and clearing the info.
        unsafe {
            for (command, target) in &self.targets {
                command.removeTarget(Some(target));
            }
            MPNowPlayingInfoCenter::defaultCenter().setNowPlayingInfo(None);
        }
    }
}
