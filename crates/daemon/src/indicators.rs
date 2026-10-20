//! Keeps the chatpad's indicator lights (Caps, Green, Orange, People) showing the state we want.
//!
//! The chatpad reports its lights in every status packet, but reads 0 while idle (backlight
//! off) even though it remembers them. A light command sent while idle only wakes the chatpad,
//! so we compare against the reported state whenever it's awake and re-send what differs.
//! While idle we only send on an actual change, so we don't keep waking it up.

use std::time::{Duration, Instant};

use x360truewireless_proto::led;

/// Minimum gap between attempts, so a chatpad that ignores us isn't flooded.
const RETRY_INTERVAL: Duration = Duration::from_millis(250);
const SWITCHABLE: u8 = led::CAPS | led::GREEN | led::ORANGE | led::PEOPLE;

#[derive(Debug, Default)]
pub struct IndicatorSync {
    /// Last status byte from the chatpad, if any.
    reported: Option<u8>,
    /// Desired state we last sent.
    sent: Option<u8>,
    last_attempt: Option<Instant>,
}

impl IndicatorSync {
    pub fn on_status(&mut self, leds: u8) {
        self.reported = Some(leds);
    }

    /// Forget everything (controller reconnected: the chatpad starts with all lights off).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Lights to switch now, as `(led bit, on)`, given the `desired` `led::*` bits.
    pub fn poll(&mut self, desired: u8, now: Instant) -> Vec<(u8, bool)> {
        let Some(reported) = self.reported else {
            return Vec::new();
        };
        let desired = desired & SWITCHABLE;
        let current = if reported & led::BACKLIGHT != 0 {
            reported & SWITCHABLE
        } else {
            // Idle: the chatpad shows nothing but remembers what we last sent (all off after
            // connecting).
            self.sent.unwrap_or(0)
        };
        if current == desired {
            return Vec::new();
        }
        if self
            .last_attempt
            .is_some_and(|t| now.duration_since(t) < RETRY_INTERVAL)
        {
            return Vec::new();
        }
        self.last_attempt = Some(now);
        self.sent = Some(desired);
        let changed = current ^ desired;
        led::ALL
            .iter()
            .filter(|&&bit| changed & bit != 0)
            .map(|&bit| (bit, desired & bit != 0))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AWAKE: u8 = led::BACKLIGHT;

    #[test]
    fn nothing_before_first_status() {
        let mut s = IndicatorSync::default();
        assert!(s.poll(led::CAPS, Instant::now()).is_empty());
    }

    #[test]
    fn awake_mismatch_is_sent_and_retried() {
        let mut s = IndicatorSync::default();
        let t = Instant::now();
        s.on_status(AWAKE);
        assert_eq!(s.poll(led::CAPS, t), vec![(led::CAPS, true)]);
        // Not confirmed yet, but too soon to retry.
        assert!(s.poll(led::CAPS, t + Duration::from_millis(100)).is_empty());
        assert_eq!(
            s.poll(led::CAPS, t + RETRY_INTERVAL),
            vec![(led::CAPS, true)]
        );
        // Confirmed.
        s.on_status(AWAKE | led::CAPS);
        assert!(s.poll(led::CAPS, t + RETRY_INTERVAL * 4).is_empty());
    }

    #[test]
    fn idle_change_sent_once_then_corrected_on_wake() {
        let mut s = IndicatorSync::default();
        let t = Instant::now();
        s.on_status(0); // idle
        assert_eq!(s.poll(led::GREEN, t), vec![(led::GREEN, true)]);
        // Still idle (the command just woke it): don't keep sending.
        assert!(s.poll(led::GREEN, t + RETRY_INTERVAL * 2).is_empty());
        // It wakes up without the light: send again.
        s.on_status(AWAKE);
        assert_eq!(
            s.poll(led::GREEN, t + RETRY_INTERVAL * 3),
            vec![(led::GREEN, true)]
        );
    }

    #[test]
    fn idle_with_nothing_to_change_stays_quiet() {
        let mut s = IndicatorSync::default();
        s.on_status(0);
        assert!(s.poll(0, Instant::now()).is_empty());
    }

    #[test]
    fn stale_light_after_wake_is_turned_off() {
        let mut s = IndicatorSync::default();
        let t = Instant::now();
        // Chatpad wakes still showing Orange from before, but we want nothing lit.
        s.on_status(AWAKE | led::ORANGE);
        assert_eq!(s.poll(0, t), vec![(led::ORANGE, false)]);
    }
}
