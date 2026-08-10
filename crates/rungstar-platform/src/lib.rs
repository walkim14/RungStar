//! Everything that touches the machine: the window, the fonts, the sound card, the
//! controllers. `rungstar-ui` describes what to draw and this turns it into pixels.

pub mod assets;
pub mod calibrate;
pub mod capture;
pub mod chiptune;
pub mod font;
pub mod input;
pub mod music;
pub mod playback;
pub mod render;
pub mod sfx;

pub use assets::asset_paths;
pub use calibrate::{measure, Calibration};
pub use capture::SdlCapture;
pub use font::{Face, FontError, FontSet};
pub use input::{Action, Bindings, InputEvent, InputMapper, Source};
pub use music::Music;
pub use playback::Playback;
pub use render::{RenderError, Renderer};
pub use sfx::{Sfx, Sound};
