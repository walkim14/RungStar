//! A cursor over a list of things, and the rules for moving it.
//!
//! Every menu in the game — main menu, options page, context menu, player setup — is a cursor
//! over N items with the same navigation rules, so those rules live here once and are tested
//! once. The rules themselves are the interesting part: wrapping at the ends, skipping rows
//! that cannot be selected, and paging by whatever fits on screen.
//!
//! Because every menu comes through here, this is also the one place that knows the cursor
//! *actually moved*, so it is where the move [`Chime`](crate::chime::Chime) is emitted. Doing
//! it at the key press instead would blip at the bottom of a list that does not wrap, and a
//! sound for a press that did nothing reads as the input having been missed — which is the
//! opposite of what it is for. Only the directional methods emit: [`Cursor::set`] is what a
//! hovering mouse and a re-sorting list call, and neither is the player moving the cursor.

use crate::chime::{emit, Chime};

/// A selection within `count` items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cursor {
    index: usize,
    count: usize,
}

impl Cursor {
    pub fn new(count: usize) -> Self {
        Self { index: 0, count }
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Change the number of items, keeping the selection if it still exists.
    pub fn set_count(&mut self, count: usize) {
        self.count = count;
        self.index = self.index.min(count.saturating_sub(1));
    }

    pub fn set(&mut self, index: usize) {
        if self.count > 0 {
            self.index = index.min(self.count - 1);
        }
    }

    /// Move by `delta`, wrapping at both ends.
    ///
    /// Menus wrap because the alternative is a dead press at the bottom of a list, and on a
    /// controller a dead press reads as the input having been missed.
    pub fn move_by(&mut self, delta: isize) {
        if self.count == 0 {
            return;
        }
        self.moved_to((self.index as isize + delta).rem_euclid(self.count as isize) as usize);
    }

    /// Move by `delta` without wrapping, stopping at the ends.
    ///
    /// Used where wrapping would be a surprise rather than a convenience: paging through a
    /// long list should stop at the end, not reappear at the top.
    pub fn move_clamped(&mut self, delta: isize) {
        if self.count == 0 {
            return;
        }
        let last = self.count as isize - 1;
        self.moved_to((self.index as isize + delta).clamp(0, last) as usize);
    }

    pub fn first(&mut self) {
        self.moved_to(0);
    }

    pub fn last(&mut self) {
        self.moved_to(self.count.saturating_sub(1));
    }

    /// Land on an index the player steered to, chiming if that is somewhere new.
    fn moved_to(&mut self, index: usize) {
        if index != self.index {
            self.index = index;
            emit(Chime::Move);
        }
    }

    /// Move to the next item `selectable` accepts, wrapping, leaving the cursor alone if
    /// nothing qualifies.
    ///
    /// Options pages have rows that are only headings, and a cursor that can land on one is a
    /// cursor that appears to have vanished.
    pub fn move_selectable(&mut self, delta: isize, selectable: impl Fn(usize) -> bool) {
        if self.count == 0 || delta == 0 {
            return;
        }
        let step = delta.signum();
        let mut candidate = self.index;
        for _ in 0..self.count {
            candidate = (candidate as isize + step).rem_euclid(self.count as isize) as usize;
            if selectable(candidate) {
                self.moved_to(candidate);
                return;
            }
        }
    }
}

/// Repeat timing for a held direction.
///
/// A held stick that moves one item per frame is unusable on a long list and a held stick that
/// moves one item per press is unusable on a long list for the opposite reason. This is the
/// usual answer: a pause, then an accelerating repeat.
#[derive(Debug, Clone)]
pub struct Repeat {
    /// Seconds held before the first repeat.
    pub delay: f32,
    /// Seconds between repeats at the start.
    pub interval: f32,
    /// The fastest repeats get, in seconds.
    pub minimum: f32,
    /// How much each repeat shortens the interval.
    pub acceleration: f32,
    held: f32,
    next: f32,
    interval_now: f32,
}

impl Default for Repeat {
    fn default() -> Self {
        Self {
            delay: 0.35,
            interval: 0.09,
            minimum: 0.02,
            acceleration: 0.88,
            held: 0.0,
            next: 0.0,
            interval_now: 0.09,
        }
    }
}

impl Repeat {
    /// Start of a press: one step immediately, then the delay before repeating.
    pub fn press(&mut self) -> usize {
        self.held = 0.0;
        self.next = self.delay;
        self.interval_now = self.interval;
        1
    }

    pub fn release(&mut self) {
        self.held = 0.0;
        self.next = 0.0;
    }

    /// Advance a held direction, returning how many steps to take this frame.
    ///
    /// Returns a count rather than a bool so a frame that ran long still scrolls the right
    /// distance: dropping the extra steps would make a stutter also lose input.
    pub fn tick(&mut self, dt: f32) -> usize {
        self.held += dt.max(0.0);
        let mut steps = 0;
        while self.held >= self.next {
            steps += 1;
            self.next += self.interval_now;
            self.interval_now = (self.interval_now * self.acceleration).max(self.minimum);
            // A frame that stalled for a second should not fire fifty steps.
            if steps >= 32 {
                break;
            }
        }
        steps
    }
}
