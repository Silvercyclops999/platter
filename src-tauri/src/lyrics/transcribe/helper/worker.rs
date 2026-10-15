//! The helper process's side: started as `platter --speech-worker` before
//! anything else in the app (no window, no tray, no single-instance lock).
//! It reads requests from stdin, runs the speech model and writes progress
//! and the result to stdout. It ends when stdin closes (the app stopped it,
//! or the app itself is gone).

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use parking_lot::Mutex;

use super::super::engine::{self, RecognizeError, Recognizer};
use super::proto::{self, Reply, Request};

enum Work {
    Run { req: Request, pcm: Vec<f32> },
    Unload,
}

type Out = Arc<Mutex<Box<dyn Write + Send>>>;

fn send(out: &Out, reply: &Reply) -> io::Result<()> {
    proto::write_msg(&mut *out.lock(), reply)
}

/// Serves requests until `input` ends. Runs happen one at a time on this
/// thread; requests are read on another so a cancel arrives mid-run.
/// `lower` is called before each run with whether to go lowest.
pub fn serve(
    mut input: Box<dyn Read + Send>,
    output: Box<dyn Write + Send>,
    rec: Arc<dyn Recognizer>,
    lower: &dyn Fn(bool),
) -> io::Result<()> {
    let out: Out = Arc::new(Mutex::new(output));
    send(&out, &Reply::Hello { version: proto::VERSION })?;
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<Work>();
    let reader = {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            loop {
                let req = match proto::read_msg::<_, Request>(&mut input) {
                    Ok(Some(r)) => r,
                    Ok(None) => break,
                    Err(e) => {
                        tracing::warn!("speech helper: bad request: {e}");
                        break;
                    }
                };
                let work = match req {
                    Request::Cancel => {
                        cancel.store(true, Ordering::Relaxed);
                        continue;
                    }
                    Request::Unload => Work::Unload,
                    Request::Transcribe { samples, .. } => {
                        let pcm = match proto::read_samples(&mut input, samples) {
                            Ok(p) => p,
                            Err(e) => {
                                tracing::warn!("speech helper: bad samples: {e}");
                                break;
                            }
                        };
                        cancel.store(false, Ordering::Relaxed);
                        Work::Run { req, pcm }
                    }
                };
                if tx.send(work).is_err() {
                    break;
                }
            }
            // The app is gone or stopped us: stop whatever runs.
            cancel.store(true, Ordering::Relaxed);
        })
    };
    let mut result = Ok(());
    for work in rx {
        let (model, kind, threads, language, lowest, pcm) = match work {
            Work::Unload => {
                rec.unload();
                continue;
            }
            Work::Run { req: Request::Transcribe { model, kind, threads, language, lowest, .. }, pcm } => {
                (model, kind, threads, language, lowest, pcm)
            }
            Work::Run { .. } => continue,
        };
        lower(lowest);
        let progress: engine::Progress = {
            let out = out.clone();
            let last = Arc::new(Mutex::new(None::<u8>));
            Arc::new(move |percent| {
                let mut l = last.lock();
                if *l != Some(percent) {
                    *l = Some(percent);
                    let _ = send(&out, &Reply::Progress { percent });
                }
            })
        };
        let reply = match rec.transcribe(&model, kind, &pcm, threads, &language, cancel.clone(), progress) {
            Ok(segments) => Reply::Done { segments },
            Err(RecognizeError::Cancelled) => Reply::Cancelled,
            Err(RecognizeError::Failed(message)) => Reply::Failed { message },
            Err(RecognizeError::Crashed) => Reply::Failed { message: "crashed".into() },
        };
        if let Err(e) = send(&out, &reply) {
            result = Err(e);
            break;
        }
    }
    drop(reader);
    result
}

/// The helper process's whole life. Returns the exit code.
pub fn main() -> i32 {
    let _ = tracing_subscriber::fmt().with_writer(io::stderr).with_ansi(false).with_target(false).try_init();
    let output = protocol_stdout();
    match serve(Box::new(io::stdin()), output, engine::in_process(), &crate::platform::cpu::lower_this_process) {
        Ok(()) => 0,
        Err(e) => {
            tracing::warn!("speech helper stopped: {e}");
            1
        }
    }
}

/// Stdout for the protocol only: on macOS and Linux anything else the
/// speech library prints to stdout goes to stderr instead. (On Windows it
/// logs to stderr already; a stray line would show up as a bad frame.)
fn protocol_stdout() -> Box<dyn Write + Send> {
    #[cfg(unix)]
    {
        use std::os::fd::FromRawFd;
        unsafe extern "C" {
            fn dup(fd: i32) -> i32;
            fn dup2(from: i32, to: i32) -> i32;
        }
        // SAFETY: plain descriptor calls on this process's own stdout and
        // stderr; the duplicate is owned by the File from here on.
        unsafe {
            let fd = dup(1);
            if fd >= 0 && dup2(2, 1) >= 0 {
                return Box::new(std::fs::File::from_raw_fd(fd));
            }
        }
    }
    Box::new(io::stdout())
}
