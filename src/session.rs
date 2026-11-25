#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_command_requires_one_exact_wake_phrase() {
        let mut session = Session::default();
        let now = std::time::Instant::now();

        assert_eq!(session.hear("open chrome", now), None);
        assert_eq!(session.hear("they said hey wiki", now), None);
        assert_eq!(session.hear("Hey Wiki!", now), Some(Event::Awake));
        assert_eq!(
            session.hear("open chrome", now),
            Some(Event::Command("open chrome".into()))
        );
        assert_eq!(session.hear("open chrome", now), None);
    }

    #[test]
    fn late_commands_are_ignored() {
        let mut session = Session::default();
        let now = std::time::Instant::now();
        assert_eq!(session.hear("hey wiki", now), Some(Event::Awake));
        assert_eq!(session.tick(now + COMMAND_WINDOW), Some(Event::TimedOut));
        assert_eq!(session.hear("open chrome", now + COMMAND_WINDOW), None);
    }

    #[test]
    fn wake_phrase_can_follow_unrecognized_background_audio() {
        let mut session = Session::default();
        let now = std::time::Instant::now();
        assert_eq!(session.hear("[unk] hey [unk]", now), None);
        assert_eq!(session.hear("[unk] hey wiki", now), Some(Event::Awake));
    }
}
use std::time::{Duration, Instant};

use crate::commands::normalize;

pub const COMMAND_WINDOW: Duration = Duration::from_secs(8);

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Awake,
    Command(String),
    TimedOut,
}

#[derive(Default)]
pub struct Session {
    deadline: Option<Instant>,
}

impl Session {
    pub fn is_awake(&self) -> bool {
        self.deadline.is_some()
    }

    pub fn hear(&mut self, text: &str, now: Instant) -> Option<Event> {
        if let Some(event) = self.tick(now) {
            return Some(event);
        }
        let phrase = normalize(text);
        if phrase.trim_start_matches("[unk] ") == "hey wiki" {
            self.deadline = Some(now + COMMAND_WINDOW);
            return Some(Event::Awake);
        }
        if self.is_awake() && !phrase.is_empty() && phrase != "[unk]" {
            self.deadline = None;
            return Some(Event::Command(text.trim().to_owned()));
        }
        None
    }

    pub fn tick(&mut self, now: Instant) -> Option<Event> {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.deadline = None;
            return Some(Event::TimedOut);
        }
        None
    }
}
