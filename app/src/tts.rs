//! Sentence-level TTS playback (mirrors the Python build's producer/player
//! split): one manager thread synthesizes queued sentences through
//! `lue_core::tts::synthesize` into rotating temp buffers and plays each via
//! an ffplay subprocess — the same playback stack the Python build uses.
//!
//! Pause aborts the current sentence and rewinds the queue to it (resume
//! replays that sentence from its start); stop clears everything.

use lue_core::tts;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TtsState {
    Idle,
    Playing,
    Paused,
}

pub enum TtsCmd {
    /// Queue of (chapter, paragraph, sentence, text) to read, in order.
    Start(Vec<(usize, usize, usize, String)>),
    Pause,
    Resume,
    Stop,
}

#[derive(Clone, Copy, Debug)]
pub enum TtsEvent {
    /// A sentence started playing; the UI scrolls to keep it visible.
    Sentence { chapter: usize, paragraph: usize },
    /// Queue drained, stopped, or synthesis failed irrecoverably.
    Finished,
}

#[derive(Clone)]
struct Job {
    chapter: usize,
    paragraph: usize,
    /// Part of the reading position; used by sentence-level highlighting
    /// once that lands, so it is carried even though the player today only
    /// follows paragraphs.
    #[allow(dead_code)]
    sentence: usize,
    text: String,
}

struct Shared {
    state: TtsState,
    queue: Vec<Job>,
    child: Option<Child>,
}

impl Shared {
    fn kill_child(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// UI-side handle: sends commands, reports coarse state, yields events.
pub struct TtsController {
    cmd_tx: Sender<TtsCmd>,
    shared: Arc<Mutex<Shared>>,
}

impl TtsController {
    /// Spawn the manager thread.
    pub fn spawn(voice: String) -> (Self, Receiver<TtsEvent>) {
        let (event_tx, event_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared {
            state: TtsState::Idle,
            queue: Vec::new(),
            child: None,
        }));
        {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("lue-tts".into())
                .spawn(move || manager_loop(cmd_rx, event_tx, voice, shared))
                .expect("spawn tts manager thread");
        }
        (Self { cmd_tx, shared }, event_rx)
    }

    pub fn send(&self, cmd: TtsCmd) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn state(&self) -> TtsState {
        self.shared
            .lock()
            .map(|s| s.state)
            .unwrap_or(TtsState::Idle)
    }

    /// The play/pause key: playing → pause, paused → resume, idle → no-op
    /// (starting always goes through [`TtsCmd::Start`] with a fresh queue).
    pub fn toggle(&self) {
        match self.state() {
            TtsState::Playing => self.send(TtsCmd::Pause),
            TtsState::Paused => self.send(TtsCmd::Resume),
            TtsState::Idle => {}
        }
    }

    pub fn is_active(&self) -> bool {
        self.state() != TtsState::Idle
    }
}

fn manager_loop(
    cmd_rx: Receiver<TtsCmd>,
    event_tx: Sender<TtsEvent>,
    voice: String,
    shared: Arc<Mutex<Shared>>,
) {
    let buffers = [
        std::env::temp_dir().join(format!("lue-tts-{}-0.mp3", std::process::id())),
        std::env::temp_dir().join(format!("lue-tts-{}-1.mp3", std::process::id())),
    ];
    let mut buffer_index = 0usize;
    'manager: loop {
        // Drain pending commands first.
        loop {
            match cmd_rx.try_recv() {
                Ok(TtsCmd::Start(jobs)) => {
                    let mut s = shared.lock().expect("tts shared");
                    s.kill_child();
                    s.queue = jobs.into_iter().map(|(c, p, se, text)| Job {
                        chapter: c,
                        paragraph: p,
                        sentence: se,
                        text,
                    }).collect();
                    s.state = TtsState::Playing;
                }
                Ok(TtsCmd::Pause) => {
                    let mut s = shared.lock().expect("tts shared");
                    if s.state == TtsState::Playing {
                        s.kill_child();
                        s.state = TtsState::Paused;
                    }
                }
                Ok(TtsCmd::Resume) => {
                    let mut s = shared.lock().expect("tts shared");
                    if s.state == TtsState::Paused {
                        s.state = TtsState::Playing;
                    }
                }
                Ok(TtsCmd::Stop) => {
                    let mut s = shared.lock().expect("tts shared");
                    s.kill_child();
                    s.queue.clear();
                    s.state = TtsState::Idle;
                    let _ = event_tx.send(TtsEvent::Finished);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        let (state, has_job) = {
            let s = shared.lock().expect("tts shared");
            (s.state, !s.queue.is_empty())
        };
        match state {
            TtsState::Playing if has_job => {}
            TtsState::Playing => {
                // Queue drained naturally.
                shared.lock().expect("tts shared").state = TtsState::Idle;
                let _ = event_tx.send(TtsEvent::Finished);
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            _ => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
        }

        let job = shared.lock().expect("tts shared").queue.first().cloned();
        let Some(job) = job else { continue };

        // Synthesize, then write to a rotating buffer file (two files keep
        // the previous one alive while ffplay may still hold it).
        let synth = tts::synthesize(&job.text, &voice);
        let buffer_path = match synth {
            Ok(mp3) => {
                let path = &buffers[buffer_index];
                buffer_index = (buffer_index + 1) % buffers.len();
                let write = std::fs::File::create(path).and_then(|mut f| {
                    f.write_all(&mp3)?;
                    f.flush()
                });
                match write {
                    Ok(()) => Some(path.clone()),
                    Err(_) => None,
                }
            }
            Err(_) => None,
        };

        let Some(path) = buffer_path else {
            // One retry, then give up and report finished (never skip the
            // whole book silently on a network failure).
            let retried = tts::synthesize(&job.text, &voice).is_ok();
            if !retried {
                let mut s = shared.lock().expect("tts shared");
                s.kill_child();
                s.queue.clear();
                s.state = TtsState::Idle;
                let _ = event_tx.send(TtsEvent::Finished);
                continue 'manager;
            }
            std::thread::sleep(Duration::from_millis(300));
            continue; // job stays at the queue front
        };

        let _ = event_tx.send(TtsEvent::Sentence {
            chapter: job.chapter,
            paragraph: job.paragraph,
        });
        {
            let mut s = shared.lock().expect("tts shared");
            s.child = Command::new("ffplay")
                .args(["-nodisp", "-autoexit", "-loglevel", "error"])
                .arg(&path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok();
        }
        // Poll for exit so Pause/Stop can kill the child mid-sentence; the
        // job stays at the queue front until it finished naturally.
        loop {
            {
                let mut s = shared.lock().expect("tts shared");
                let done = match s.child.as_mut() {
                    Some(child) => matches!(child.try_wait(), Ok(Some(_)) | Err(_)),
                    None => true,
                };
                if done {
                    s.child = None;
                    if s.state == TtsState::Playing {
                        s.queue.remove(0);
                    }
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_start_reports_finished_and_idle() {
        let (controller, rx) = TtsController::spawn("test-voice".into());
        controller.send(TtsCmd::Start(vec![]));
        let event = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("manager should report finished for an empty queue");
        assert!(matches!(event, TtsEvent::Finished));
        assert_eq!(controller.state(), TtsState::Idle);
    }

    #[test]
    fn pause_and_stop_when_idle_stay_idle() {
        let (controller, _rx) = TtsController::spawn("test-voice".into());
        controller.send(TtsCmd::Pause);
        controller.send(TtsCmd::Stop);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(controller.state(), TtsState::Idle);
        controller.toggle(); // must not panic and must stay idle
        assert_eq!(controller.state(), TtsState::Idle);
    }
}
