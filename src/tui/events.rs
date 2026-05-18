use crossterm::event::{self, Event, KeyEvent};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

pub enum AppEvent {
    Key(KeyEvent),
    Tick,
}

pub fn spawn_event_thread(tx: Sender<AppEvent>) {
    std::thread::Builder::new()
        .name("screwdriver-events".into())
        .spawn(move || {
            let tick_rate = Duration::from_millis(50);
            let mut last_tick = Instant::now();
            loop {
                let timeout = tick_rate
                    .checked_sub(last_tick.elapsed())
                    .unwrap_or(Duration::ZERO);

                if event::poll(timeout).unwrap_or(false) {
                    if let Ok(Event::Key(key)) = event::read() {
                        if tx.send(AppEvent::Key(key)).is_err() {
                            break;
                        }
                    }
                }

                if last_tick.elapsed() >= tick_rate {
                    if tx.send(AppEvent::Tick).is_err() {
                        break;
                    }
                    last_tick = Instant::now();
                }
            }
        })
        .expect("failed to spawn event thread");
}
