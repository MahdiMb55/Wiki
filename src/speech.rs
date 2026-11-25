use std::{
    ffi::CString,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use cpal::{
    FromSample, SampleFormat, SampleRate, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use wiki::{
    commands::Commands,
    session::{Event, Session},
};

use crate::{
    actions,
    vosk::{Model, Recognizer},
};

const WAKE_GRAMMAR: &std::ffi::CStr = c"[\"hey wiki\", \"[unk]\"]";

#[derive(Clone)]
pub enum Update {
    Waiting,
    Awake,
    Outcome { transcript: String, message: String },
    Error(String),
}

pub struct Speech {
    stop: Arc<AtomicBool>,
    pub updates: Receiver<Update>,
}

impl Speech {
    pub fn start(
        commands: Arc<RwLock<Commands>>,
        refresh: impl Fn(&Update) + Send + 'static,
    ) -> Result<Self> {
        let (send, updates) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        thread::Builder::new()
            .name("wiki-speech".into())
            .spawn(move || {
                let notify = |update: Update| {
                    if !worker_stop.load(Ordering::Acquire) && send.send(update.clone()).is_ok() {
                        refresh(&update);
                    }
                };
                if let Err(err) = listen(&worker_stop, &commands, &notify) {
                    notify(Update::Error(format!("{err:#}")));
                }
            })?;
        Ok(Self { stop, updates })
    }
}

impl Drop for Speech {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn listen(stop: &AtomicBool, commands: &RwLock<Commands>, notify: &impl Fn(Update)) -> Result<()> {
    let exe = std::env::current_exe()?;
    let model = Model::load(exe.parent().context("Wiki's app folder is unavailable.")?)?;
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    let Microphone {
        stream,
        rate,
        samples,
        errors,
    } = microphone()?;
    let mut decoder = Decoder::new(&model, rate)?;
    stream.play().context("Could not start the microphone.")?;
    notify(Update::Waiting);
    while !stop.load(Ordering::Acquire) {
        if let Ok(error) = errors.try_recv() {
            bail!("Microphone stopped: {error}");
        }
        if let Some(event) = decoder.tick(Instant::now())? {
            update(event, commands, notify);
        }
        if let Ok(audio) = samples.recv_timeout(Duration::from_millis(30))
            && !stop.load(Ordering::Acquire)
            && let Some(event) = decoder.feed(&audio, Instant::now(), commands)?
        {
            update(event, commands, notify);
        }
    }
    Ok(())
}

fn update(event: Event, commands: &RwLock<Commands>, notify: &impl Fn(Update)) {
    match event {
        Event::Awake => notify(Update::Awake),
        Event::TimedOut => notify(Update::Waiting),
        Event::Command(text) => {
            let commands = commands.read().expect("command lock poisoned");
            let message = match commands.resolve(&text) {
                Some(resolved) => {
                    let phrase = resolved.command.phrase.clone();
                    actions::perform(resolved)
                        .map_or_else(|err| format!("{err:#}"), |_| format!("Ran “{phrase}”."))
                }
                None => format!("Heard “{text}”. No matching command."),
            };
            notify(Update::Outcome {
                transcript: text,
                message,
            });
        }
    }
}

struct Decoder<'a> {
    model: &'a Model,
    rate: u32,
    recognizer: Recognizer<'a>,
    session: Session,
    command_audio: Vec<i16>,
}

impl<'a> Decoder<'a> {
    fn new(model: &'a Model, rate: u32) -> Result<Self> {
        Ok(Self {
            model,
            rate,
            recognizer: model.recognizer(rate, Some(WAKE_GRAMMAR))?,
            session: Session::default(),
            command_audio: Vec::new(),
        })
    }

    fn tick(&mut self, now: Instant) -> Result<Option<Event>> {
        let event = self.session.tick(now);
        if event.is_some() {
            self.command_audio.clear();
            self.recognizer = self.model.recognizer(self.rate, Some(WAKE_GRAMMAR))?;
        }
        Ok(event)
    }

    fn feed(
        &mut self,
        audio: &[i16],
        now: Instant,
        commands: &RwLock<Commands>,
    ) -> Result<Option<Event>> {
        if self.session.is_awake() {
            self.command_audio.extend_from_slice(audio);
        }
        let Some(mut text) = self.recognizer.accept(audio)? else {
            return Ok(None);
        };
        if self.session.is_awake() && !text.is_empty() {
            let commands = commands.read().expect("command lock poisoned");
            if commands.resolve(&text).is_none()
                && let Some(alternative) = self.try_fixed_phrases(&commands)?
            {
                text = alternative;
            }
        }
        self.command_audio.clear();
        let event = self.session.hear(&text, now);
        if event.is_some() {
            let grammar = (!self.session.is_awake()).then_some(WAKE_GRAMMAR);
            self.recognizer = self.model.recognizer(self.rate, grammar)?;
        }
        Ok(event)
    }

    fn try_fixed_phrases(&self, commands: &Commands) -> Result<Option<String>> {
        let mut phrases: Vec<_> = commands
            .fixed_speech_phrases()
            .filter(|phrase| {
                phrase
                    .split_whitespace()
                    .all(|word| self.model.contains_word(word))
            })
            .collect();
        if phrases.is_empty() {
            return Ok(None);
        }
        phrases.push("[unk]".into());
        let grammar = CString::new(serde_json::to_string(&phrases)?)?;
        let mut recognizer = self.model.recognizer(self.rate, Some(&grammar))?;
        let text = match recognizer.accept(&self.command_audio)? {
            Some(text) => text,
            None => recognizer.finish()?,
        };
        Ok(commands.resolve(&text).is_some().then_some(text))
    }
}

struct Microphone {
    stream: Stream,
    rate: u32,
    samples: Receiver<Vec<i16>>,
    errors: Receiver<cpal::StreamError>,
}

fn microphone() -> Result<Microphone> {
    let device = cpal::default_host().default_input_device().context(
        "No default microphone found. Connect a microphone and check Windows sound settings.",
    )?;
    let supported = device
        .default_input_config()
        .context("Could not configure the microphone.")?;
    let config = supported.config();
    let SampleRate(rate) = config.sample_rate;
    let (audio, samples) = mpsc::sync_channel(50);
    let (send_error, errors) = mpsc::channel();
    let error = move |err| {
        let _ = send_error.send(err);
    };
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, audio, error),
        SampleFormat::I16 => build::<i16>(&device, &config, audio, error),
        SampleFormat::I32 => build::<i32>(&device, &config, audio, error),
        SampleFormat::U16 => build::<u16>(&device, &config, audio, error),
        format => bail!("Unsupported microphone format: {format}."),
    }
    .context("Could not open the microphone. Check Windows microphone privacy settings.")?;
    Ok(Microphone {
        stream,
        rate,
        samples,
        errors,
    })
}

fn build<T: SizedSample>(
    device: &cpal::Device,
    config: &StreamConfig,
    audio: SyncSender<Vec<i16>>,
    error: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream, cpal::BuildStreamError>
where
    i16: FromSample<T>,
{
    let channels = usize::from(config.channels);
    device.build_input_stream(
        config,
        move |samples: &[T], _| {
            let mono = samples
                .chunks_exact(channels)
                .map(|frame| {
                    let sum: i64 = frame
                        .iter()
                        .map(|sample| i64::from(sample.to_sample::<i16>()))
                        .sum();
                    (sum / channels as i64) as i16
                })
                .collect();
            let _ = audio.try_send(mono);
        },
        error,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    #[ignore = "requires a prepared Vosk bundle and 16-bit mono PCM files in WIKI_SPEECH_FIXTURES"]
    fn recognizes_wake_and_command_from_audio() -> Result<()> {
        let directory = std::env::var_os("WIKI_SPEECH_BUNDLE").context("Set WIKI_SPEECH_BUNDLE")?;
        let fixtures =
            std::env::var_os("WIKI_SPEECH_FIXTURES").context("Set WIKI_SPEECH_FIXTURES")?;
        let fixtures = Path::new(&fixtures);
        let model = Model::load(Path::new(&directory))?;
        let mut decoder = Decoder::new(&model, 48_000)?;
        let commands = RwLock::new(Commands::default());
        let mut now = Instant::now();
        let mut play = |name: &str| -> Result<Vec<Event>> {
            let audio = std::fs::read(fixtures.join(name))?;
            let (pcm, remainder) = audio.as_chunks::<2>();
            assert!(remainder.is_empty(), "Malformed PCM in {name}");
            let samples: Vec<_> = pcm.iter().copied().map(i16::from_le_bytes).collect();
            let mut events = Vec::new();
            for chunk in samples.chunks(480) {
                now += Duration::from_millis(10);
                events.extend(decoder.feed(chunk, now, &commands)?);
            }
            Ok(events)
        };
        assert!(play("chrome.pcm")?.is_empty());
        assert!(play("hello.pcm")?.is_empty());
        assert_eq!(play("wake.pcm")?, [Event::Awake]);
        assert_eq!(play("chrome.pcm")?, [Event::Command("open chrome".into())]);
        assert!(play("chrome.pcm")?.is_empty());
        let defaults = Commands::default();
        for (file, expected) in [
            ("chatgpt.pcm", "Open ChatGPT"),
            ("media.pcm", "Play or pause"),
            ("search.pcm", "Search for"),
            ("switch.pcm", "Switch apps"),
        ] {
            assert_eq!(play("wake.pcm")?, [Event::Awake], "wake before {file}");
            let events = play(file)?;
            let [Event::Command(ref text)] = events[..] else {
                bail!("Expected one command from {file}, got {events:?}");
            };
            let resolved = defaults
                .resolve(text)
                .with_context(|| format!("{file}: heard {text:?}"))?;
            assert_eq!(resolved.command.phrase, expected);
            if file == "search.pcm" {
                assert_eq!(resolved.argument, "rust programming");
            }
        }
        Ok(())
    }
}
