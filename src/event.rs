//! Event loop and input handling

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use crossterm::event::{self, Event, MouseEvent, MouseEventKind};

const EVENT_POLL_TIMEOUT: Duration = Duration::from_millis(100);
const MAX_EVENTS_PER_BATCH: usize = 512;
const MAX_BUFFERED_EVENTS: usize = MAX_EVENTS_PER_BATCH * 2;
const SCROLL_BURST_GAP: Duration = Duration::from_millis(1);
const MAX_EVENTS_PER_NOTCH: usize = 8;

#[derive(Debug, Default)]
pub struct EventBatch {
    events: Vec<InputEvent>,
    raw_count: usize,
    retained_count: usize,
}

impl EventBatch {
    pub fn had_input(&self) -> bool {
        self.raw_count > 0 || self.retained_count > 0
    }

    pub fn raw_count(&self) -> usize {
        self.raw_count
    }

    pub fn retained_count(&self) -> usize {
        self.retained_count
    }

    pub fn into_events(self) -> Vec<InputEvent> {
        self.events
    }
}

#[derive(Debug)]
pub enum InputEvent {
    Terminal(Event),
    Scroll { mouse: MouseEvent, steps: usize },
}

#[cfg(test)]
#[derive(Clone)]
struct TimedEvent {
    event: Event,
    arrived_at: Instant,
}

enum InputMessage {
    Event(Event),
    Error(String),
}

pub struct EventReader {
    receiver: Receiver<InputMessage>,
    raw_count: Arc<AtomicUsize>,
}

impl EventReader {
    pub fn new() -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(MAX_BUFFERED_EVENTS);
        let raw_count = Arc::new(AtomicUsize::new(0));
        let reader_raw_count = Arc::clone(&raw_count);
        let _reader_thread = thread::Builder::new()
            .name("keifu-input".to_string())
            .spawn(move || {
                let mut scroll = ScrollNormalizer::default();
                loop {
                    match event::read() {
                        Ok(event) => {
                            reader_raw_count.fetch_add(1, Ordering::Relaxed);
                            let Some(event) =
                                normalize_terminal_event(&mut scroll, event, Instant::now())
                            else {
                                continue;
                            };
                            if enqueue_event(&sender, event).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = sender.send(InputMessage::Error(error.to_string()));
                            break;
                        }
                    }
                }
            })
            .context("Failed to start terminal input reader")?;

        Ok(Self {
            receiver,
            raw_count,
        })
    }

    pub fn poll_events(&mut self) -> Result<EventBatch> {
        self.poll_events_with_timeout(EVENT_POLL_TIMEOUT)
    }

    /// Wait at most `timeout`; real input wakes the channel immediately.
    pub fn poll_events_with_timeout(&mut self, timeout: Duration) -> Result<EventBatch> {
        let first = match self.receiver.recv_timeout(timeout) {
            Ok(message) => message,
            Err(RecvTimeoutError::Timeout) => {
                return Ok(EventBatch {
                    raw_count: self.raw_count.swap(0, Ordering::Relaxed),
                    ..EventBatch::default()
                });
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(anyhow!("Terminal input reader stopped unexpectedly"));
            }
        };

        let first = Self::event_from_message(first)?;
        let mut events = Vec::with_capacity(16);
        let mut retained_count = push_input_event(first, &mut events);
        let mut message_count = 1;
        while message_count < MAX_EVENTS_PER_BATCH {
            match self.receiver.try_recv() {
                Ok(message) => {
                    message_count += 1;
                    let event = Self::event_from_message(message)?;
                    retained_count += push_input_event(event, &mut events);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return Err(anyhow!("Terminal input reader stopped unexpectedly"));
                }
            }
        }

        Ok(EventBatch {
            events,
            raw_count: self.raw_count.swap(0, Ordering::Relaxed),
            retained_count,
        })
    }

    fn event_from_message(message: InputMessage) -> Result<Event> {
        match message {
            InputMessage::Event(event) => Ok(event),
            InputMessage::Error(error) => Err(anyhow!("Terminal input failed: {error}")),
        }
    }
}

fn enqueue_event(sender: &SyncSender<InputMessage>, event: Event) -> std::result::Result<(), ()> {
    let is_scroll_event = matches!(&event, Event::Mouse(mouse) if is_scroll(mouse.kind));
    let message = InputMessage::Event(event);
    if is_scroll_event {
        match sender.try_send(message) {
            Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
            Err(TrySendError::Disconnected(_)) => Err(()),
        }
    } else {
        sender.send(message).map_err(|_| ())
    }
}

fn normalize_terminal_event(
    scroll: &mut ScrollNormalizer,
    event: Event,
    arrived_at: Instant,
) -> Option<Event> {
    match event {
        Event::Mouse(mouse) if is_scroll(mouse.kind) => scroll
            .should_retain(mouse, arrived_at)
            .then_some(Event::Mouse(mouse)),
        other => {
            scroll.reset();
            Some(other)
        }
    }
}

fn push_input_event(event: Event, events: &mut Vec<InputEvent>) -> usize {
    match event {
        Event::Mouse(mouse) if is_scroll(mouse.kind) => match events.last_mut() {
            Some(InputEvent::Scroll {
                mouse: previous,
                steps,
            }) if *previous == mouse => *steps += 1,
            _ => events.push(InputEvent::Scroll { mouse, steps: 1 }),
        },
        other => events.push(InputEvent::Terminal(other)),
    }
    1
}

fn is_scroll(kind: MouseEventKind) -> bool {
    matches!(kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown)
}

#[derive(Debug, Default)]
struct ScrollNormalizer {
    last_scroll: Option<(Instant, MouseEvent)>,
    burst_count: usize,
}

impl ScrollNormalizer {
    fn should_retain(&mut self, mouse: MouseEvent, arrived_at: Instant) -> bool {
        let continues_burst = self.last_scroll.is_some_and(|(last, last_mouse)| {
            last_mouse == mouse && arrived_at.saturating_duration_since(last) < SCROLL_BURST_GAP
        });
        self.last_scroll = Some((arrived_at, mouse));

        if !continues_burst {
            self.burst_count = 1;
            return true;
        }

        self.burst_count += 1;
        if self.burst_count > MAX_EVENTS_PER_NOTCH {
            self.burst_count = 1;
            true
        } else {
            false
        }
    }

    fn reset(&mut self) {
        self.last_scroll = None;
        self.burst_count = 0;
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

    use super::*;

    fn scroll(kind: MouseEventKind, arrived_at: Instant) -> TimedEvent {
        TimedEvent {
            event: Event::Mouse(MouseEvent {
                kind,
                column: 5,
                row: 5,
                modifiers: KeyModifiers::NONE,
            }),
            arrived_at,
        }
    }

    fn paced_scrolls(
        kind: MouseEventKind,
        count: usize,
        start: Instant,
        gap: Duration,
    ) -> Vec<TimedEvent> {
        (0..count)
            .map(|index| scroll(kind, start + gap * index as u32))
            .collect()
    }

    fn retained_count(events: &[InputEvent]) -> usize {
        events
            .iter()
            .map(|event| match event {
                InputEvent::Terminal(_) => 1,
                InputEvent::Scroll { steps, .. } => *steps,
            })
            .sum()
    }

    fn normalize(
        scroll: &mut ScrollNormalizer,
        events: impl IntoIterator<Item = TimedEvent>,
    ) -> Vec<InputEvent> {
        let mut normalized = Vec::new();
        for timed in events {
            if let Some(event) = normalize_terminal_event(scroll, timed.event, timed.arrived_at) {
                push_input_event(event, &mut normalized);
            }
        }
        normalized
    }

    #[test]
    fn normalizes_amplified_events_from_one_notch() {
        let now = Instant::now();
        for count in [1, 3, MAX_EVENTS_PER_NOTCH] {
            let mut normalizer = ScrollNormalizer::default();
            let events = paced_scrolls(
                MouseEventKind::ScrollDown,
                count,
                now,
                Duration::from_micros(50),
            );

            let normalized = normalize(&mut normalizer, events);

            assert_eq!(retained_count(&normalized), 1);
        }
    }

    #[test]
    fn keeps_each_amplified_notch_in_a_fast_gesture() {
        let now = Instant::now();
        let events = (0..20).flat_map(|notch| {
            paced_scrolls(
                MouseEventKind::ScrollDown,
                MAX_EVENTS_PER_NOTCH,
                now + Duration::from_millis(notch * 4),
                Duration::from_micros(50),
            )
        });
        let mut normalizer = ScrollNormalizer::default();

        let normalized = normalize(&mut normalizer, events);

        assert_eq!(retained_count(&normalized), 20);
    }

    #[test]
    fn retains_fast_scroll_distance_independent_of_render_batches() {
        let now = Instant::now();
        let events = paced_scrolls(
            MouseEventKind::ScrollDown,
            60,
            now,
            Duration::from_millis(4),
        );
        let mut one_batch = ScrollNormalizer::default();
        let normalized = normalize(&mut one_batch, events);
        let one_batch_total = retained_count(&normalized);
        assert!(matches!(
            normalized.as_slice(),
            [InputEvent::Scroll { steps: 60, .. }]
        ));

        let mut split_batches = ScrollNormalizer::default();
        let split_total: usize = paced_scrolls(
            MouseEventKind::ScrollDown,
            60,
            now,
            Duration::from_millis(4),
        )
        .chunks(5)
        .map(|batch| retained_count(&normalize(&mut split_batches, batch.to_vec())))
        .sum();

        assert_eq!(one_batch_total, 60);
        assert_eq!(split_total, 60);
    }

    #[test]
    fn bounds_a_sub_millisecond_free_spin_stream() {
        let now = Instant::now();
        let mut normalizer = ScrollNormalizer::default();
        let events = paced_scrolls(
            MouseEventKind::ScrollDown,
            80,
            now,
            Duration::from_micros(50),
        );

        let normalized = normalize(&mut normalizer, events);

        assert_eq!(retained_count(&normalized), 10);
    }

    #[test]
    fn keeps_direction_changes_and_other_events_in_order() {
        let now = Instant::now();
        let mut normalizer = ScrollNormalizer::default();
        let key = TimedEvent {
            event: Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
            arrived_at: now + Duration::from_micros(200),
        };
        let events = vec![
            scroll(MouseEventKind::ScrollDown, now),
            scroll(MouseEventKind::ScrollDown, now + Duration::from_micros(50)),
            key,
            scroll(MouseEventKind::ScrollUp, now + Duration::from_micros(250)),
            scroll(MouseEventKind::ScrollUp, now + Duration::from_micros(300)),
        ];

        let normalized = normalize(&mut normalizer, events);

        assert_eq!(retained_count(&normalized), 3);
        assert!(matches!(
            normalized.as_slice(),
            [
                InputEvent::Scroll {
                    mouse: MouseEvent {
                        kind: MouseEventKind::ScrollDown,
                        ..
                    },
                    steps: 1
                },
                InputEvent::Terminal(Event::Key(_)),
                InputEvent::Scroll {
                    mouse: MouseEvent {
                        kind: MouseEventKind::ScrollUp,
                        ..
                    },
                    steps: 1
                }
            ]
        ));
    }

    #[test]
    fn keeps_scrolls_when_the_pointer_or_modifiers_change() {
        let now = Instant::now();
        let mut normalizer = ScrollNormalizer::default();
        let mut moved = scroll(MouseEventKind::ScrollDown, now + Duration::from_micros(50));
        let Event::Mouse(mouse) = &mut moved.event else {
            unreachable!();
        };
        mouse.column += 1;
        let mut modified = scroll(MouseEventKind::ScrollDown, now + Duration::from_micros(100));
        let Event::Mouse(mouse) = &mut modified.event else {
            unreachable!();
        };
        mouse.modifiers = KeyModifiers::SHIFT;

        let normalized = normalize(
            &mut normalizer,
            vec![scroll(MouseEventKind::ScrollDown, now), moved, modified],
        );

        assert_eq!(retained_count(&normalized), 3);
    }

    #[test]
    fn one_millisecond_gap_starts_a_new_notch() {
        let now = Instant::now();
        let mut within_gap = ScrollNormalizer::default();
        let within = normalize(
            &mut within_gap,
            vec![
                scroll(MouseEventKind::ScrollDown, now),
                scroll(MouseEventKind::ScrollDown, now + Duration::from_micros(999)),
            ],
        );
        let mut at_boundary = ScrollNormalizer::default();
        let boundary = normalize(
            &mut at_boundary,
            vec![
                scroll(MouseEventKind::ScrollDown, now),
                scroll(MouseEventKind::ScrollDown, now + Duration::from_millis(1)),
            ],
        );

        assert_eq!(retained_count(&within), 1);
        assert_eq!(retained_count(&boundary), 2);
    }

    #[test]
    fn full_queue_drops_scroll_but_preserves_keyboard_input() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let scroll = scroll(MouseEventKind::ScrollDown, Instant::now()).event;
        assert_eq!(enqueue_event(&sender, scroll.clone()), Ok(()));
        assert_eq!(enqueue_event(&sender, scroll), Ok(()));

        let key = Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        let key_sender = sender.clone();
        let send_key = thread::spawn(move || enqueue_event(&key_sender, key));

        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(InputMessage::Event(Event::Mouse(_)))
        ));
        assert_eq!(send_key.join().unwrap(), Ok(()));
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(InputMessage::Event(Event::Key(_)))
        ));
    }


    #[test]
    fn idle_timeout_returns_an_empty_batch() {
        let (_sender, receiver) = mpsc::sync_channel(1);
        let mut reader = EventReader {
            receiver,
            raw_count: Arc::new(AtomicUsize::new(0)),
        };
        let batch = reader.poll_events_with_timeout(Duration::ZERO).unwrap();
        assert!(!batch.had_input());
        assert!(batch.into_events().is_empty());
    }

    #[test]
    fn queued_input_wakes_a_long_idle_wait_immediately() {
        let (sender, receiver) = mpsc::sync_channel(1);
        sender.send(InputMessage::Event(Event::Key(
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        ))).unwrap();
        let mut reader = EventReader {
            receiver,
            raw_count: Arc::new(AtomicUsize::new(1)),
        };
        let batch = reader.poll_events_with_timeout(Duration::from_secs(60)).unwrap();
        assert!(batch.had_input());
        assert_eq!(batch.retained_count(), 1);
        assert!(matches!(batch.into_events().as_slice(), [InputEvent::Terminal(Event::Key(_))]));
    }

    #[test]
    fn disconnected_input_returns_an_error_instead_of_spinning() {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(sender);
        let mut reader = EventReader {
            receiver,
            raw_count: Arc::new(AtomicUsize::new(0)),
        };
        assert!(reader.poll_events_with_timeout(Duration::ZERO).is_err());
    }
}
