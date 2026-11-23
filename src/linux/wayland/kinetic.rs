//! Kinetic scrolling for a touchpad, added by Ritornel to GPUI's Wayland
//! backend (`README.md` at the root of this repository). It depends on
//! nothing but the standard library, so that its tests run with `rustc
//! --test` alone: this crate's own tests need GPUI's test support, which
//! the published crate leaves out.
//!
//! A compositor sends a touchpad's scroll while the fingers move and an
//! `axis_stop` when they lift, and nothing after: the toolkit coasts. This
//! does it with the numbers GTK 4 (4.20) uses in `GtkScrolledWindow`, so
//! the page moves on as a GTK app's does. It is written from what GTK does,
//! not from its code:
//!
//! - The velocity at the lift is the distance scrolled in the last 150 ms
//!   before it, the lift included, over that time, so fingers that stop
//!   before lifting throw nothing.
//! - The coast then slows exponentially at a rate of 4 a second, and ends
//!   when it is below a tenth of a pixel a second.
//! - A flick that catches the last one's coast, the same way and at least
//!   a third as fast, adds up to six times the speed that coast had when
//!   caught, so that flick after flick goes faster. Fingers that catch a
//!   coast and lift without scrolling forget it; a coast a click stopped
//!   counts at the speed it would have slowed to by the next lift.
//!
//! The distances are GPUI's: it moves a page three pixels for each unit a
//! touchpad scrolls, where GTK moves it two and a half, so a flick goes as
//! far, for the scroll that threw it, as in GTK.

use std::collections::VecDeque;
use std::time::Instant;

/// How far back the velocity at a lift looks, in milliseconds.
const HISTORY_MS: u32 = 150;
/// How fast a coast slows: its velocity falls by `e` every quarter second.
const FRICTION: f64 = 4.;
/// The speed, in pixels a second, below which a coast has stopped.
const STOPPED: f64 = 0.1;
/// A flick adds the last one's speed when it is at least this part of it...
const ADD_FROM: f64 = 0.33;
/// ...adding all of it when it is this part of it...
const ADD_ALL_AT: f64 = 1.;
/// ...and at most this many times it.
const ADD_AT_MOST: f64 = 6.;

#[derive(Debug, Default)]
pub(crate) struct Kinetic {
    /// The scroll's movements in the last `HISTORY_MS`, by the compositor's
    /// clock in milliseconds.
    history: VecDeque<(u32, [f64; 2])>,
    coast: Option<Coast>,
    /// The last coast, once stopped, which the next flick may add to.
    stopped: Option<Stopped>,
}

/// How a coast was stopped, and so what the next flick may add of it.
#[derive(Debug)]
enum Stopped {
    /// Caught by fingers on the touchpad: its speed then.
    Caught([f64; 2]),
    /// Ended by a click or a pinch: its speed slows on, unseen.
    Ended(Coast),
}

#[derive(Debug)]
struct Coast {
    start: Instant,
    velocity: [f64; 2],
    /// How far it has moved since `start`.
    travelled: [f64; 2],
}

impl Coast {
    /// Where the coast is `seconds` after it started, and how fast it goes.
    fn at(&self, seconds: f64) -> ([f64; 2], [f64; 2]) {
        let left = (-FRICTION * seconds).exp();
        let axis = |v: f64| (v / FRICTION * (1. - left), v * left);
        let (x, vx) = axis(self.velocity[0]);
        let (y, vy) = axis(self.velocity[1]);
        ([x, y], [vx, vy])
    }
}

impl Kinetic {
    /// The fingers moved the page `delta`, across and down, at `time`.
    pub(crate) fn scroll(&mut self, time: u32, delta: [f64; 2]) {
        while self
            .history
            .front()
            .is_some_and(|(at, _)| time.wrapping_sub(*at) > HISTORY_MS)
        {
            self.history.pop_front();
        }
        self.history.push_back((time, delta));
    }

    /// The fingers lifted at `time`: starts a coast if they were moving,
    /// and says whether it did.
    pub(crate) fn lift(&mut self, time: u32, now: Instant) -> bool {
        self.scroll(time, [0., 0.]);
        let first = self.history.front().map_or(time, |(at, _)| *at);
        let span = time.wrapping_sub(first);
        let mut velocity = [0., 0.];
        if span > 0 {
            for (_, delta) in self.history.drain(..) {
                velocity[0] += delta[0];
                velocity[1] += delta[1];
            }
            velocity = velocity.map(|distance| distance * 1000. / f64::from(span));
        }
        self.history.clear();
        let before = match self.stopped.take() {
            Some(Stopped::Caught(velocity)) => velocity,
            Some(Stopped::Ended(coast)) => {
                coast.at(now.duration_since(coast.start).as_secs_f64()).1
            }
            None => [0., 0.],
        };
        self.coast = None;
        if velocity == [0., 0.] {
            return false;
        }
        for (velocity, before) in velocity.iter_mut().zip(before) {
            *velocity += added(*velocity, before);
        }
        self.coast = Some(Coast {
            start: now,
            velocity,
            travelled: [0., 0.],
        });
        true
    }

    /// Fingers on the touchpad catch the coast: it stops, and a flick
    /// that follows may add the speed it had.
    pub(crate) fn catch(&mut self, now: Instant) {
        if let Some(coast) = self.coast.take() {
            let velocity = coast.at(now.duration_since(coast.start).as_secs_f64()).1;
            self.stopped = Some(Stopped::Caught(velocity));
        }
    }

    /// Fingers that caught a coast lifted without scrolling: its speed is
    /// forgotten.
    pub(crate) fn let_go(&mut self) {
        if matches!(self.stopped, Some(Stopped::Caught(_))) {
            self.stopped = None;
        }
    }

    /// A click, a pinch or a mouse wheel ends the coast: it stops, and a
    /// later flick may add the speed it would have slowed to by then. A
    /// coast fingers caught and kept is forgotten.
    pub(crate) fn end(&mut self) {
        match self.coast.take() {
            Some(coast) => self.stopped = Some(Stopped::Ended(coast)),
            None => self.let_go(),
        }
    }

    /// How far the coast moves the page, across and down, from its last
    /// step to `now`, and whether it goes on after this step.
    pub(crate) fn step(&mut self, now: Instant) -> Option<([f64; 2], bool)> {
        let coast = self.coast.as_mut()?;
        let (position, velocity) = coast.at(now.duration_since(coast.start).as_secs_f64());
        let delta = [
            position[0] - coast.travelled[0],
            position[1] - coast.travelled[1],
        ];
        coast.travelled = position;
        let going = velocity.iter().any(|v| v.abs() >= STOPPED);
        if !going {
            self.coast = None;
        }
        Some((delta, going))
    }
}

/// What a flick at `velocity` gains from one at `before` it interrupted, on
/// one axis.
fn added(velocity: f64, before: f64) -> f64 {
    let same_way = (velocity >= 0.) == (before >= 0.);
    if before == 0. || !same_way {
        return 0.;
    }
    // How fast this flick is, as a part of the last one's speed.
    let ratio = velocity / before;
    if ratio < ADD_FROM {
        return 0.;
    }
    let share = (ratio - ADD_FROM) / (ADD_ALL_AT - ADD_FROM);
    before * share.min(ADD_AT_MOST)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn down(kinetic: &mut Kinetic, from: u32, steps: u32, each: f64) {
        for step in 0..steps {
            kinetic.scroll(from + step * 10, [0., each]);
        }
    }

    #[test]
    fn a_flick_coasts_as_far_as_its_speed_over_the_friction() {
        let mut kinetic = Kinetic::default();
        // 20 px every 10 ms, lifted 10 ms after the last: 2,000 px/s.
        down(&mut kinetic, 0, 20, 20.);
        let start = Instant::now();
        assert!(kinetic.lift(200, start));
        // Only the last 150 ms count: 15 moves of 20 over 150 ms.
        let mut travelled = 0.;
        let mut at = start;
        while let Some((delta, going)) = kinetic.step(at) {
            travelled += delta[1];
            if !going {
                break;
            }
            at += Duration::from_millis(16);
        }
        assert!((travelled - 2000. / 4.).abs() < 1., "{travelled}");
        assert!(kinetic.coast.is_none());
        // It stops when below a tenth of a pixel a second.
        let seconds = at.duration_since(start).as_secs_f64();
        assert!(
            (seconds - (2000f64 / 0.1).ln() / 4.).abs() < 0.02,
            "{seconds}"
        );
    }

    #[test]
    fn fingers_that_rest_before_lifting_throw_nothing() {
        let mut kinetic = Kinetic::default();
        down(&mut kinetic, 0, 10, 20.);
        assert!(!kinetic.lift(90 + 200, Instant::now()));
        assert!(kinetic.step(Instant::now()).is_none());
    }

    /// A flick of 2,000 px/s lifted at `lift`, then the speed of the coast
    /// it starts.
    fn flick(kinetic: &mut Kinetic, lift: u32, now: Instant) -> Option<f64> {
        down(kinetic, lift - 160, 16, 20.);
        kinetic.lift(lift, now);
        kinetic.coast.as_ref().map(|coast| coast.at(0.).1[1])
    }

    #[test]
    fn a_flick_that_catches_a_coast_adds_its_speed() {
        let mut kinetic = Kinetic::default();
        let start = Instant::now();
        assert_eq!(flick(&mut kinetic, 160, start), Some(2000.));
        kinetic.catch(start);
        // As fast as the last: it gains all of it, so twice as fast.
        let later = start + Duration::from_millis(300);
        let speed = flick(&mut kinetic, 460, later);
        assert!(
            speed.is_some_and(|speed| (speed - 4000.).abs() < 1.),
            "{speed:?}"
        );
    }

    #[test]
    fn a_coast_caught_then_let_go_or_ended_adds_nothing() {
        let mut kinetic = Kinetic::default();
        let start = Instant::now();
        flick(&mut kinetic, 160, start);
        kinetic.catch(start);
        kinetic.let_go();
        let speed = flick(&mut kinetic, 60_160, start + Duration::from_secs(60));
        assert_eq!(speed, Some(2000.));
        // Caught, then ended by a wheel or a click: forgotten as well.
        kinetic.catch(start + Duration::from_secs(60));
        kinetic.end();
        let speed = flick(&mut kinetic, 120_160, start + Duration::from_secs(120));
        assert_eq!(speed, Some(2000.));
    }

    #[test]
    fn a_coast_a_click_ended_adds_what_it_has_slowed_to() {
        let mut kinetic = Kinetic::default();
        let start = Instant::now();
        flick(&mut kinetic, 160, start);
        kinetic.end();
        // A quarter second on, it would be going 2000/e, which 2,000 px/s
        // is 2.72 times: it adds (2.72 - 0.33) / 0.67 of that, about 2,620.
        let soon = flick(&mut kinetic, 410, start + Duration::from_millis(250));
        let expected = 2000. + 2000. / 1f64.exp() * (1f64.exp() - 0.33) / 0.67;
        assert!(
            soon.is_some_and(|speed| (speed - expected).abs() < 1.),
            "{soon:?}"
        );
        kinetic.end();
        // A minute on, it would have stopped long since.
        let later = flick(&mut kinetic, 60_410, start + Duration::from_secs(60));
        assert!(
            later.is_some_and(|speed| (speed - 2000.).abs() < 1e-6),
            "{later:?}"
        );
    }

    #[test]
    fn a_flick_the_other_way_or_much_slower_gains_nothing() {
        assert_eq!(added(-1000., 1000.), 0.);
        assert_eq!(added(300., 1000.), 0.);
        assert_eq!(added(10_000., 1000.), 6000.);
        assert_eq!(added(500., 0.), 0.);
    }
}
