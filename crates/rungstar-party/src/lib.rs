//! Game modes: challenges, parties and tournaments.
//!
//! UltraStar Deluxe implements its challenge modes as **Lua plugins** — fourteen `.usdx` files
//! in `game/plugins/`, each re-implementing the same "walk the lines, compare the scores" loop
//! against a scripting API. Cutting the scripting engine would silently cut the party modes, so
//! they are reimplemented natively instead. Nothing is lost: they are data — some combination
//! of *hide something*, *stop early* and *put somebody out* — and the loop that reads that data
//! lives once, in [`watch`].
//!
//! What that buys is testability. A Lua plugin can only be tested by singing a song; a
//! [`Watch`] is fed beats and scores by a test in a millisecond.
//!
//! Nothing in this crate knows about audio, drawing, files or the time of day. The one place
//! that needs randomness — the Deaf mode cutting the music in and out — takes a seed.

pub mod bracket;
pub mod challenge;
pub mod party;
pub mod watch;

pub use bracket::{Bracket, BracketError, Match};
pub use challenge::{Challenge, Effects, Finish, Knockout, Length, Music};
pub use party::{Party, Phase, Played, Team};
pub use watch::{Ending, Standing, Watch};
