//! Turning device events into things the game means.
//!
//! Screens never see a key code or a button index. They see [`Action::Confirm`], and it does
//! not matter whether that arrived from Return, the South button on a gamepad, or a tap. That
//! indirection is the whole reason the game can be played from the sofa: UltraStar Deluxe
//! wires one controller to fake keystrokes, so anything the keyboard cannot express — per
//! player controllers, rebinding, rumble — simply cannot exist there.
//!
//! Gamepad buttons are named by position, not by letter. "South" is A on an Xbox pad and B on
//! a Nintendo one; binding to the position means the button under your thumb does the same
//! thing on every controller.

use std::collections::HashMap;

use sdl3::gamepad::{Axis, Button};
use sdl3::keyboard::Keycode;

/// Something the player wants to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    /// Accept, select, start singing.
    Confirm,
    /// Go back one screen.
    Back,
    /// Open the context menu for whatever is selected.
    Menu,
    /// Jump to the search field.
    Search,
    PageUp,
    PageDown,
    /// Pick something at random.
    Random,
    ToggleFavourite,
    Pause,
    Screenshot,
    ToggleFullscreen,
    /// Leave the game entirely.
    Quit,
}

/// Where an action came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    Keyboard,
    /// The instance id of the gamepad, so party modes can tell players apart.
    Gamepad(u32),
    Mouse,
}

/// An action, and whether it was pressed or released.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputEvent {
    pub action: Action,
    pub source: Source,
    /// Which player this controller belongs to, once they have been assigned.
    pub player: Option<u8>,
    pub pressed: bool,
    /// Whether this is a key auto-repeat rather than a fresh press.
    pub repeat: bool,
}

/// How far a stick must be pushed before it counts as a direction.
///
/// Sticks rest slightly off centre and drift as they wear; without a dead zone a worn
/// controller scrolls a song list on its own.
const STICK_THRESHOLD: f32 = 0.5;

/// Which key and button does what.
#[derive(Debug, Clone)]
pub struct Bindings {
    keyboard: HashMap<Keycode, Action>,
    gamepad: HashMap<Button, Action>,
}

impl Bindings {
    /// The out-of-the-box mapping.
    pub fn standard() -> Self {
        let keyboard = HashMap::from([
            (Keycode::Up, Action::Up),
            (Keycode::Down, Action::Down),
            (Keycode::Left, Action::Left),
            (Keycode::Right, Action::Right),
            (Keycode::Return, Action::Confirm),
            (Keycode::KpEnter, Action::Confirm),
            (Keycode::Space, Action::Confirm),
            (Keycode::Escape, Action::Back),
            (Keycode::Backspace, Action::Back),
            (Keycode::M, Action::Menu),
            (Keycode::J, Action::Search),
            (Keycode::PageUp, Action::PageUp),
            (Keycode::PageDown, Action::PageDown),
            (Keycode::R, Action::Random),
            (Keycode::F, Action::ToggleFavourite),
            (Keycode::P, Action::Pause),
            (Keycode::F12, Action::Screenshot),
            (Keycode::F11, Action::ToggleFullscreen),
        ]);
        let gamepad = HashMap::from([
            (Button::DPadUp, Action::Up),
            (Button::DPadDown, Action::Down),
            (Button::DPadLeft, Action::Left),
            (Button::DPadRight, Action::Right),
            // Position, not letter: this is the button under the right thumb everywhere.
            (Button::South, Action::Confirm),
            (Button::East, Action::Back),
            (Button::West, Action::Search),
            (Button::North, Action::Menu),
            (Button::LeftShoulder, Action::PageUp),
            (Button::RightShoulder, Action::PageDown),
            (Button::Start, Action::Pause),
            (Button::Back, Action::Menu),
        ]);
        Self { keyboard, gamepad }
    }
}

impl Bindings {
    pub fn action_for_key(&self, key: Keycode) -> Option<Action> {
        self.keyboard.get(&key).copied()
    }

    pub fn action_for_button(&self, button: Button) -> Option<Action> {
        self.gamepad.get(&button).copied()
    }

    /// Rebind a key, replacing any previous meaning it had.
    pub fn bind_key(&mut self, key: Keycode, action: Action) {
        self.keyboard.insert(key, action);
    }

    pub fn bind_button(&mut self, button: Button, action: Action) {
        self.gamepad.insert(button, action);
    }

    /// Every key currently bound to an action, for showing in a rebinding screen.
    pub fn keys_for(&self, action: Action) -> Vec<Keycode> {
        self.keyboard
            .iter()
            .filter(|(_, a)| **a == action)
            .map(|(k, _)| *k)
            .collect()
    }
}

impl Default for Bindings {
    fn default() -> Self {
        Self::standard()
    }
}

/// Which direction a stick is currently pushed, so motion becomes discrete presses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct StickState {
    horizontal: Option<Action>,
    vertical: Option<Action>,
}

/// Translates device events into [`InputEvent`]s.
#[derive(Debug, Default)]
pub struct InputMapper {
    bindings: Bindings,
    sticks: HashMap<u32, StickState>,
    players: HashMap<u32, u8>,
}

impl InputMapper {
    pub fn new(bindings: Bindings) -> Self {
        Self {
            bindings,
            sticks: HashMap::new(),
            players: HashMap::new(),
        }
    }

    pub fn bindings(&self) -> &Bindings {
        &self.bindings
    }

    pub fn bindings_mut(&mut self) -> &mut Bindings {
        &mut self.bindings
    }

    /// Give a controller to a player, so party modes know who pressed what.
    pub fn assign_player(&mut self, gamepad: u32, player: u8) {
        self.players.insert(gamepad, player);
    }

    pub fn player_of(&self, gamepad: u32) -> Option<u8> {
        self.players.get(&gamepad).copied()
    }

    pub fn forget_gamepad(&mut self, gamepad: u32) {
        self.sticks.remove(&gamepad);
        self.players.remove(&gamepad);
    }

    pub fn key(&self, key: Keycode, pressed: bool, repeat: bool) -> Option<InputEvent> {
        Some(InputEvent {
            action: self.bindings.action_for_key(key)?,
            source: Source::Keyboard,
            player: None,
            pressed,
            repeat,
        })
    }

    pub fn button(&self, gamepad: u32, button: Button, pressed: bool) -> Option<InputEvent> {
        Some(InputEvent {
            action: self.bindings.action_for_button(button)?,
            source: Source::Gamepad(gamepad),
            player: self.player_of(gamepad),
            pressed,
            repeat: false,
        })
    }

    /// Convert stick movement into direction presses and releases.
    ///
    /// Returns both events when a stick is swept straight across: the release of the old
    /// direction, then the press of the new one.
    pub fn axis(&mut self, gamepad: u32, axis: Axis, value: i16) -> Vec<InputEvent> {
        let (negative, positive) = match axis {
            Axis::LeftX | Axis::RightX => (Action::Left, Action::Right),
            Axis::LeftY | Axis::RightY => (Action::Up, Action::Down),
            // Triggers rest at one end of their range, so they are not directions.
            _ => return Vec::new(),
        };
        let normalised = f32::from(value) / f32::from(i16::MAX);
        let direction = if normalised <= -STICK_THRESHOLD {
            Some(negative)
        } else if normalised >= STICK_THRESHOLD {
            Some(positive)
        } else {
            None
        };

        let player = self.players.get(&gamepad).copied();
        let state = self.sticks.entry(gamepad).or_default();
        let slot = match axis {
            Axis::LeftX | Axis::RightX => &mut state.horizontal,
            _ => &mut state.vertical,
        };
        if *slot == direction {
            return Vec::new();
        }
        let previous = std::mem::replace(slot, direction);

        let mut events = Vec::new();
        if let Some(previous) = previous {
            events.push(InputEvent {
                action: previous,
                source: Source::Gamepad(gamepad),
                player,
                pressed: false,
                repeat: false,
            });
        }
        if let Some(current) = direction {
            events.push(InputEvent {
                action: current,
                source: Source::Gamepad(gamepad),
                player,
                pressed: true,
                repeat: false,
            });
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gamepad_and_a_keyboard_produce_the_same_action() {
        let mapper = InputMapper::default();
        assert_eq!(
            mapper.key(Keycode::Return, true, false).unwrap().action,
            Action::Confirm
        );
        assert_eq!(
            mapper.button(0, Button::South, true).unwrap().action,
            Action::Confirm
        );
    }

    #[test]
    fn unbound_input_produces_nothing() {
        let mapper = InputMapper::default();
        assert!(mapper.key(Keycode::F7, true, false).is_none());
    }

    #[test]
    fn a_stick_inside_the_dead_zone_is_ignored() {
        let mut mapper = InputMapper::default();
        assert!(
            mapper.axis(0, Axis::LeftY, 8_000).is_empty(),
            "drift must not scroll"
        );
    }

    #[test]
    fn pushing_a_stick_presses_once_not_continuously() {
        let mut mapper = InputMapper::default();
        let first = mapper.axis(0, Axis::LeftY, 30_000);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].action, Action::Down);
        assert!(first[0].pressed);

        // Still held: no further events, or the song list would race past.
        assert!(mapper.axis(0, Axis::LeftY, 32_000).is_empty());

        let released = mapper.axis(0, Axis::LeftY, 0);
        assert_eq!(released.len(), 1);
        assert!(!released[0].pressed);
    }

    #[test]
    fn sweeping_a_stick_across_releases_then_presses() {
        let mut mapper = InputMapper::default();
        mapper.axis(0, Axis::LeftX, -30_000);
        let swept = mapper.axis(0, Axis::LeftX, 30_000);
        assert_eq!(swept.len(), 2);
        assert_eq!((swept[0].action, swept[0].pressed), (Action::Left, false));
        assert_eq!((swept[1].action, swept[1].pressed), (Action::Right, true));
    }

    #[test]
    fn the_two_stick_axes_are_tracked_separately() {
        let mut mapper = InputMapper::default();
        mapper.axis(0, Axis::LeftX, 30_000);
        let vertical = mapper.axis(0, Axis::LeftY, 30_000);
        assert_eq!(vertical.len(), 1, "pushing down must not cancel right");
        assert_eq!(vertical[0].action, Action::Down);
    }

    #[test]
    fn events_carry_the_player_the_controller_belongs_to() {
        let mut mapper = InputMapper::default();
        mapper.assign_player(3, 2);
        let event = mapper.button(3, Button::South, true).unwrap();
        assert_eq!(event.player, Some(2));
        assert_eq!(event.source, Source::Gamepad(3));

        mapper.forget_gamepad(3);
        assert_eq!(mapper.button(3, Button::South, true).unwrap().player, None);
    }

    #[test]
    fn rebinding_replaces_the_previous_meaning() {
        let mut mapper = InputMapper::default();
        mapper
            .bindings_mut()
            .bind_key(Keycode::Return, Action::Back);
        assert_eq!(
            mapper.key(Keycode::Return, true, false).unwrap().action,
            Action::Back
        );
        // Space was also bound to Confirm and is untouched.
        assert_eq!(
            mapper.key(Keycode::Space, true, false).unwrap().action,
            Action::Confirm
        );
    }

    #[test]
    fn each_pad_keeps_its_own_stick_state() {
        let mut mapper = InputMapper::default();
        mapper.axis(0, Axis::LeftY, 30_000);
        let second = mapper.axis(1, Axis::LeftY, 30_000);
        assert_eq!(second.len(), 1, "a second player's stick is independent");
    }
}
