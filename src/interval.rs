//! Implements a copy of tokios interval functionality in burst mode using os sleep primitives

use std::{
    thread::{park, sleep},
    time::{Duration, Instant},
};

pub struct Interval {
    next: Option<Instant>,
    period: Duration,
}

impl Interval {
    /// Immediately arms interval
    pub fn new(period: Duration) -> Self {
        assert!(period > Duration::ZERO, "`period` must be non-zero.");

        Self {
            period,
            next: Some(Instant::now()),
        }
    }

    pub fn tick(&mut self) -> Instant {
        match self.next {
            Some(goal) => {
                let now = Instant::now();

                if goal > now {
                    sleep(goal - now);
                }

                self.next = goal.checked_add(self.period);

                goal
            }
            // next overflowed, sleep eternally
            None => loop {
                park();
            },
        }
    }
}
