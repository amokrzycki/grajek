use crate::player::{Phase, State};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visual {
    Idle,
    Playing,
    Paused,
    Error,
}

impl From<&State> for Visual {
    fn from(state: &State) -> Self {
        match state.phase {
            Phase::Playing => Self::Playing,
            Phase::Paused => Self::Paused,
            Phase::Error => Self::Error,
            Phase::Initializing
            | Phase::Ready
            | Phase::Connecting
            | Phase::Resolving
            | Phase::Stopping
            | Phase::Disconnecting => Self::Idle,
        }
    }
}

impl Visual {
    pub fn pngs(self) -> [&'static [u8]; SIZES.len()] {
        macro_rules! sizes {
            ($state:literal) => {
                [
                    include_bytes!(concat!("../../../assets/icons/tray-", $state, "-16.png"))
                        .as_slice(),
                    include_bytes!(concat!("../../../assets/icons/tray-", $state, "-22.png"))
                        .as_slice(),
                    include_bytes!(concat!("../../../assets/icons/tray-", $state, "-32.png"))
                        .as_slice(),
                    include_bytes!(concat!("../../../assets/icons/tray-", $state, "-48.png"))
                        .as_slice(),
                ]
            };
        }
        match self {
            Self::Idle => sizes!("idle"),
            Self::Playing => sizes!("playing"),
            Self::Paused => sizes!("paused"),
            Self::Error => sizes!("error"),
        }
    }

    #[cfg_attr(
        not(any(target_os = "windows", test)),
        expect(
            dead_code,
            reason = "Only the Windows tray collapses the PNG set to one bitmap"
        )
    )]
    pub fn png(self) -> &'static [u8] {
        self.pngs()[WINDOWS_PNG]
    }
}

pub const SIZES: [i32; 4] = [16, 22, 32, 48];
#[cfg_attr(
    not(any(target_os = "windows", test)),
    expect(
        dead_code,
        reason = "Index into the shared size table for the Windows bitmap"
    )
)]
const WINDOWS_PNG: usize = 3;

pub fn decode(png: &[u8]) -> image::ImageResult<image::RgbaImage> {
    image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map(image::DynamicImage::into_rgba8)
}

#[cfg(target_os = "linux")]
pub fn argb(mut rgba: Vec<u8>) -> Vec<u8> {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.rotate_right(1);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{Phase, State};
    use crate::playlist::ImportStatus;

    #[test]
    fn every_controller_phase_selects_its_icon_at_startup() {
        for (phase, expected) in [
            (Phase::Initializing, Visual::Idle),
            (Phase::Ready, Visual::Idle),
            (Phase::Connecting, Visual::Idle),
            (Phase::Resolving, Visual::Idle),
            (Phase::Playing, Visual::Playing),
            (Phase::Paused, Visual::Paused),
            (Phase::Stopping, Visual::Idle),
            (Phase::Disconnecting, Visual::Idle),
            (Phase::Error, Visual::Error),
        ] {
            let state = State {
                phase,
                ..State::default()
            };
            assert_eq!(Visual::from(&state), expected);
        }
    }

    #[test]
    fn local_collection_feedback_and_progress_do_not_change_the_icon() {
        let mut state = State {
            phase: Phase::Playing,
            ..State::default()
        };
        state.position = std::time::Duration::from_secs(42);
        state.import = ImportStatus::Failed("Import failed".into());
        state.error = Some("Previous track failed".into());
        state.favorites.push(crate::favorites::FavoriteEntry {
            url: "https://www.youtube.com/watch?v=aaaaaaaaaaa".into(),
            title: Some("Saved track".into()),
            channel: None,
            duration_secs: None,
            live: false,
        });
        for result in [Ok(()), Err(crate::favorites::FavoriteRefusal::Full)] {
            state.favorite_result = Some(crate::favorites::FavoriteResult {
                request_id: 1,
                url: state.favorites[0].url.clone(),
                saved: true,
                result,
            });
            assert_eq!(Visual::from(&state), Visual::Playing);
        }
        state.favorites.clear();
        assert_eq!(Visual::from(&state), Visual::Playing);
        state.phase = Phase::Paused;
        assert_eq!(Visual::from(&state), Visual::Paused);
    }

    #[test]
    fn embedded_pngs_preserve_dimensions_palette_and_straight_alpha() -> anyhow::Result<()> {
        for (visual, colors) in [
            (
                Visual::Idle,
                vec![[25, 27, 24], [239, 237, 229], [221, 194, 139]],
            ),
            (Visual::Playing, vec![[221, 194, 139]]),
            (Visual::Paused, vec![[173, 174, 163], [221, 194, 139]]),
            (Visual::Error, vec![[173, 174, 163], [237, 169, 155]]),
        ] {
            for (png, size) in visual.pngs().into_iter().zip(SIZES) {
                let size = u32::try_from(size)?;
                let bitmap = decode(png)?;
                assert_eq!(bitmap.dimensions(), (size, size));
                assert_eq!(bitmap.as_raw().len(), (size * size * 4) as usize);
                assert_eq!(bitmap.get_pixel(0, 0).0, [0, 0, 0, 0]);
                assert!(bitmap.pixels().any(|pixel| (1..255).contains(&pixel[3])));
                for color in colors.iter().copied() {
                    assert!(
                        bitmap
                            .pixels()
                            .any(|pixel| pixel.0 == [color[0], color[1], color[2], 255])
                    );
                }
            }
            assert_eq!(visual.png(), visual.pngs()[WINDOWS_PNG]);
            assert_eq!(SIZES[WINDOWS_PNG], 48);
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dbus_argb_keeps_alpha_unpremultiplied_and_channels_in_network_order() {
        assert_eq!(
            argb(vec![221, 194, 139, 64, 237, 169, 155, 255, 0, 0, 0, 0]),
            [64, 221, 194, 139, 255, 237, 169, 155, 0, 0, 0, 0]
        );
    }

    #[test]
    fn antialiased_edges_keep_the_supplied_rgba_values() -> anyhow::Result<()> {
        for (visual, x, y, expected) in [
            (Visual::Idle, 1, 0, [26, 26, 23, 98]),
            (Visual::Playing, 5, 2, [222, 194, 139, 160]),
            (Visual::Paused, 5, 2, [222, 194, 139, 160]),
            (Visual::Error, 11, 2, [236, 168, 155, 41]),
        ] {
            assert_eq!(decode(visual.pngs()[0])?.get_pixel(x, y).0, expected);
        }
        Ok(())
    }
}
