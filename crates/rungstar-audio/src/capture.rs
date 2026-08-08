//! Getting each singer's voice onto its own analysis buffer.
//!
//! Karaoke microphone hardware is peculiar. The cheap USB adapters everyone owns present
//! themselves as a single stereo input device with one microphone on the left channel and one
//! on the right, so reaching four singers means two devices and four channels, not four
//! devices. Anything that assumes one device per player cannot support the hardware people
//! actually have.
//!
//! So the mapping is per *channel*: every channel of every device names the player it feeds,
//! or nobody.

use std::collections::BTreeMap;

/// A channel mapped to this feeds nobody.
pub const CHANNEL_OFF: u8 = 0;

/// Highest player count the game supports.
///
/// UltraStar Deluxe's arrays go to twelve, but its own source notes that eight and twelve
/// "have never worked at any point in history". Six is the honest number.
pub const MAX_PLAYERS: usize = 6;

/// Ask the backend to pick a latency.
pub const LATENCY_AUTODETECT: i32 = -1;

/// One capture device and what its channels are for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceConfig {
    /// Name as reported by the audio backend, used to find it again after a restart.
    pub name: String,
    /// Which recording source on the device, for hardware that exposes several.
    pub input_index: u32,
    /// Requested latency in milliseconds, or [`LATENCY_AUTODETECT`].
    pub latency_ms: i32,
    /// One entry per channel: [`CHANNEL_OFF`], or a one-based player number.
    pub channel_to_player: Vec<u8>,
}

impl DeviceConfig {
    /// A device with every channel disabled.
    pub fn silent(name: impl Into<String>, channels: usize) -> Self {
        Self {
            name: name.into(),
            input_index: 0,
            latency_ms: LATENCY_AUTODETECT,
            channel_to_player: vec![CHANNEL_OFF; channels],
        }
    }

    pub fn channels(&self) -> usize {
        self.channel_to_player.len()
    }

    /// Assign a channel to a one-based player number.
    pub fn assign(&mut self, channel: usize, player: u8) {
        if let Some(slot) = self.channel_to_player.get_mut(channel) {
            *slot = player;
        }
    }
}

/// Something wrong with a capture setup, in terms a settings screen can show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigProblem {
    /// A player has no channel feeding them, so they cannot score at all.
    PlayerHasNoInput { player: u8 },
    /// Two channels feed one player; whichever is louder would win at random.
    PlayerHasSeveralInputs { player: u8, channels: usize },
    /// A channel names a player outside the supported range.
    PlayerOutOfRange { player: u8 },
}

/// Check a capture setup against the number of players about to sing.
///
/// Worth doing up front: a player silently mapped to nothing scores zero for a whole song,
/// and from the sofa that is indistinguishable from a broken microphone.
pub fn validate(devices: &[DeviceConfig], player_count: usize) -> Vec<ConfigProblem> {
    let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
    let mut problems = Vec::new();

    for device in devices {
        for &player in &device.channel_to_player {
            if player == CHANNEL_OFF {
                continue;
            }
            if usize::from(player) > MAX_PLAYERS {
                problems.push(ConfigProblem::PlayerOutOfRange { player });
                continue;
            }
            *counts.entry(player).or_default() += 1;
        }
    }

    for player in 1..=player_count as u8 {
        match counts.get(&player).copied().unwrap_or(0) {
            0 => problems.push(ConfigProblem::PlayerHasNoInput { player }),
            1 => {}
            channels => {
                problems.push(ConfigProblem::PlayerHasSeveralInputs { player, channels });
            }
        }
    }
    problems
}

/// Per-player mono buffers, filled by de-interleaving each device's capture block.
///
/// Grown on demand rather than fixed at [`MAX_PLAYERS`]. Six is the limit on *singers*, not on
/// inputs: the microphone setup screen listens to every channel of every device at once so it
/// can show a meter for each, and a machine with four capture devices has more channels than
/// singers. Capping the buffers at six silently merged everything past the sixth into one
/// slot — two microphones sharing a meter — and left the readers beyond it with nothing.
#[derive(Debug, Clone, Default)]
pub struct PlayerBuffers {
    buffers: Vec<Vec<i16>>,
}

impl PlayerBuffers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Samples captured for a one-based player number.
    pub fn player(&self, player: u8) -> &[i16] {
        let index = usize::from(player).saturating_sub(1);
        self.buffers.get(index).map_or(&[][..], Vec::as_slice)
    }

    /// Drop everything captured so far, keeping the allocations.
    pub fn clear(&mut self) {
        for buffer in &mut self.buffers {
            buffer.clear();
        }
    }

    /// Split one device's interleaved capture block out to the players it feeds.
    ///
    /// A partial frame at the end of the block is ignored rather than padded: the missing
    /// samples arrive in the next block, and inventing zeroes would put a click in the middle
    /// of a held note.
    pub fn route(&mut self, config: &DeviceConfig, interleaved: &[i16]) {
        let channels = config.channels();
        if channels == 0 {
            return;
        }
        let frames = interleaved.len() / channels;

        for (channel, &player) in config.channel_to_player.iter().enumerate() {
            if player == CHANNEL_OFF {
                continue;
            }
            let slot = usize::from(player) - 1;
            if slot >= self.buffers.len() {
                self.buffers.resize_with(slot + 1, Vec::new);
            }
            let buffer = &mut self.buffers[slot];
            buffer.reserve(frames);
            for frame in 0..frames {
                buffer.push(interleaved[frame * channels + channel]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The standard cheap adapter: one stereo device, two singers.
    fn dual_mic_adapter() -> DeviceConfig {
        DeviceConfig {
            name: "USB Audio Device".to_owned(),
            input_index: 0,
            latency_ms: LATENCY_AUTODETECT,
            channel_to_player: vec![1, 2],
        }
    }

    #[test]
    fn a_stereo_device_feeds_two_players() {
        let config = dual_mic_adapter();
        let mut buffers = PlayerBuffers::new();
        // Interleaved left/right: left counts up, right counts down.
        buffers.route(&config, &[10, -10, 20, -20, 30, -30]);

        assert_eq!(buffers.player(1), &[10, 20, 30]);
        assert_eq!(buffers.player(2), &[-10, -20, -30]);
    }

    #[test]
    fn four_players_across_two_stereo_devices() {
        let first = dual_mic_adapter();
        let mut second = dual_mic_adapter();
        second.name = "USB Audio Device #2".to_owned();
        second.channel_to_player = vec![3, 4];

        let mut buffers = PlayerBuffers::new();
        buffers.route(&first, &[1, 2, 1, 2]);
        buffers.route(&second, &[3, 4, 3, 4]);

        assert_eq!(buffers.player(1), &[1, 1]);
        assert_eq!(buffers.player(2), &[2, 2]);
        assert_eq!(buffers.player(3), &[3, 3]);
        assert_eq!(buffers.player(4), &[4, 4]);
        assert!(validate(&[first, second], 4).is_empty());
    }

    #[test]
    fn a_disabled_channel_is_discarded() {
        let mut config = dual_mic_adapter();
        config.channel_to_player = vec![1, CHANNEL_OFF];
        let mut buffers = PlayerBuffers::new();
        buffers.route(&config, &[7, 99, 8, 99]);

        assert_eq!(buffers.player(1), &[7, 8]);
        assert!(buffers.player(2).is_empty());
    }

    #[test]
    fn a_partial_frame_waits_for_the_rest() {
        let config = dual_mic_adapter();
        let mut buffers = PlayerBuffers::new();
        // Five samples across two channels: the trailing sample has no partner yet.
        buffers.route(&config, &[1, 2, 3, 4, 5]);
        assert_eq!(buffers.player(1), &[1, 3]);
        assert_eq!(buffers.player(2), &[2, 4]);
    }

    #[test]
    fn an_unmapped_player_is_reported() {
        let config = dual_mic_adapter();
        let problems = validate(&[config], 3);
        assert_eq!(
            problems,
            vec![ConfigProblem::PlayerHasNoInput { player: 3 }]
        );
    }

    #[test]
    fn a_doubly_mapped_player_is_reported() {
        let mut config = dual_mic_adapter();
        config.channel_to_player = vec![1, 1];
        let problems = validate(&[config], 1);
        assert_eq!(
            problems,
            vec![ConfigProblem::PlayerHasSeveralInputs {
                player: 1,
                channels: 2
            }]
        );
    }

    #[test]
    fn buffers_accumulate_across_blocks() {
        let config = dual_mic_adapter();
        let mut buffers = PlayerBuffers::new();
        buffers.route(&config, &[1, 2]);
        buffers.route(&config, &[3, 4]);
        assert_eq!(buffers.player(1), &[1, 3]);
        buffers.clear();
        assert!(buffers.player(1).is_empty());
    }

    #[test]
    fn a_six_channel_interface_can_feed_every_player() {
        let config = DeviceConfig {
            name: "Focusrite".to_owned(),
            input_index: 0,
            latency_ms: 10,
            channel_to_player: vec![1, 2, 3, 4, 5, 6],
        };
        let mut buffers = PlayerBuffers::new();
        buffers.route(&config, &[1, 2, 3, 4, 5, 6]);
        for player in 1..=6u8 {
            assert_eq!(buffers.player(player), &[i16::from(player)]);
        }
        assert!(validate(&[config], 6).is_empty());
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;

    /// Two stereo microphones and a stereo headset: six channels, and a fourth device takes
    /// it past the singer limit.
    fn device(name: &str, first_slot: u8) -> DeviceConfig {
        DeviceConfig {
            name: name.to_owned(),
            input_index: 0,
            latency_ms: LATENCY_AUTODETECT,
            channel_to_player: vec![first_slot, first_slot + 1],
        }
    }

    #[test]
    fn more_inputs_than_singers_do_not_share_a_buffer() {
        // The reported symptom: with several capture devices connected, one microphone shared
        // a meter with another and the ones after it said nothing was arriving. The buffers
        // were a fixed six — the limit on singers, wrongly applied to inputs.
        let mut buffers = PlayerBuffers::new();
        let devices = [
            device("karaoke one", 1),
            device("karaoke two", 3),
            device("auna", 5),
            device("headset", 7),
        ];
        // Each device sends a block whose samples identify it, so a mix-up is visible.
        for (index, config) in devices.iter().enumerate() {
            let left = (index as i16 + 1) * 100;
            let right = left + 1;
            buffers.route(config, &[left, right, left, right]);
        }

        for slot in 1..=8u8 {
            let samples = buffers.player(slot);
            assert_eq!(
                samples.len(),
                2,
                "slot {slot} received {} samples",
                samples.len()
            );
            let expected = ((slot as i16 - 1) / 2 + 1) * 100 + ((slot as i16 - 1) % 2);
            assert!(
                samples.iter().all(|s| *s == expected),
                "slot {slot} holds {samples:?}, expected {expected} — inputs were combined"
            );
        }
    }

    #[test]
    fn a_channel_switched_off_stays_empty() {
        let mut buffers = PlayerBuffers::new();
        let config = DeviceConfig {
            name: "one live channel".to_owned(),
            input_index: 0,
            latency_ms: LATENCY_AUTODETECT,
            channel_to_player: vec![1, CHANNEL_OFF],
        };
        buffers.route(&config, &[10, 20, 10, 20]);
        assert_eq!(buffers.player(1), &[10, 10]);
        assert!(
            buffers.player(2).is_empty(),
            "an off channel wrote somewhere"
        );
    }
}
