//! What the interface just did, so something else can make a noise about it.
//!
//! A `Chime` is an *event*, not a sound: no file name, no volume, no audio API. That is what
//! lets it live in this crate, which has no graphics or audio in it and is tested without a
//! window. `rungstar-platform` decides what a `Chime::Move` sounds like, and a test asserts
//! that moving the cursor emitted one — which is a stronger check than listening to it.
//!
//! It is a queue rather than a return value because the alternative is threading a feedback
//! parameter through every `handle` in the crate, and every screen added after this one.
//! Nothing reads the queue except the frame loop, which drains it once and plays the result,
//! so a screen's behaviour cannot depend on it and the purity that makes screens testable is
//! intact.
//!
//! **Only deliberate movement chimes.** Moving the cursor with a stick or a key emits;
//! hovering with the mouse, or a list re-sorting under a cursor that stays put, does not. A
//! sound for something the player did not do reads as a fault.

use std::cell::RefCell;

/// Something worth hearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Chime {
    /// The cursor moved to a different item.
    Move,
    /// Something was chosen.
    Select,
    /// A screen or a menu was left.
    Back,
    /// A song is starting.
    Start,
    /// A golden note is being hit.
    Golden,
    /// A line was sung well.
    Line,
    /// A song ended.
    Finish,
    /// Something was refused.
    No,
}

thread_local! {
    static PENDING: RefCell<Vec<Chime>> = const { RefCell::new(Vec::new()) };
}

/// Say that something happened.
pub fn emit(chime: Chime) {
    PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        // A frame that emits the same chime twice should not play it twice: a Confirm that
        // both selects an item and lands a cursor somewhere is one event to the player. The
        // cap is a backstop for a frame nobody drains — a headless test, say — so this cannot
        // grow without bound.
        if !pending.contains(&chime) && pending.len() < 16 {
            pending.push(chime);
        }
    });
}

/// Take everything that has happened since the last call.
///
/// Sorted, so a frame carrying several is played in the order the variants are declared —
/// which is roughly least to most significant, and means the interesting one lands last.
pub fn take() -> Vec<Chime> {
    PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        let mut out = std::mem::take(&mut *pending);
        out.sort_unstable();
        out
    })
}

/// Throw away anything pending.
///
/// Used when a screen opens or closes: the sounds of getting there have already played, and
/// what a transition queued up on the way out should not arrive over the top of the new screen.
pub fn clear() {
    PENDING.with(|pending| pending.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_chime_twice_in_a_frame_is_one_chime() {
        clear();
        emit(Chime::Move);
        emit(Chime::Move);
        assert_eq!(take(), vec![Chime::Move]);
    }

    #[test]
    fn draining_empties_the_queue() {
        clear();
        emit(Chime::Select);
        assert_eq!(take(), vec![Chime::Select]);
        assert!(take().is_empty());
    }

    #[test]
    fn a_frame_carrying_several_orders_them() {
        clear();
        emit(Chime::Select);
        emit(Chime::Move);
        assert_eq!(take(), vec![Chime::Move, Chime::Select]);
    }
}
