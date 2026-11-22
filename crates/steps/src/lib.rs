//! The steps recipes are made of, and the built-in recipes.
//!
//! For now every step is simulated (`sim`): it behaves like the real one,
//! with the timings and the outcomes of a real disc, so the engine and any
//! window can be built and tested without a drive. Real steps replace them
//! kind by kind, under the same names, so recipes don't change.

pub mod choose;
pub mod game_sim;
pub mod hold;
pub mod map;
pub mod real;
pub mod recipes;
pub mod sim;
pub mod template;

use drt_engine::Registry;

/// Every step, simulated. `bad_track` is a track Rip fails to verify on
/// the first read, to exercise retries.
/// Every step, for real: a drive, cyanrip, MusicBrainz, ffmpeg and files.
pub fn real() -> Registry {
    let mut r = Registry::default();
    real::register(&mut r);
    r
}

pub fn simulated(bad_track: Option<u32>) -> Registry {
    simulated_bad(bad_track.into_iter().collect())
}

/// Every step, simulated, with several tracks Rip fails to verify.
pub fn simulated_bad(bad_tracks: Vec<u32>) -> Registry {
    let mut r = Registry::default();
    sim::register(&mut r, bad_tracks);
    r
}
