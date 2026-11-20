//! The built-in recipes.

use drt_engine::recipe::{branches, chain, group};
use drt_engine::{Node, Recipe};

/// The simulated drives. The built-in recipes read from these; on real
/// hardware they're not connected until you pick yours on a recipe's Drive
/// tile.
pub const BLU_RAY: &str = "Blu-ray drive";
pub const DVD: &str = "DVD drive";

/// CD to my library: FLAC through a review folder into the library (held
/// for your approval), MP3 straight into its library, and, as soon as the
/// last track is read, the tray opened and back to the start for the next
/// disc.
pub fn cd() -> Recipe {
    Recipe {
        name: "CDs".into(),
        root: chain([
            Node::step("drive", "drive").with("drive", BLU_RAY),
            Node::step("cd", "audio-cd"),
            branches([Node::step("mb", "musicbrainz"), Node::step("cdtext", "cd-text")]),
            Node::step("choose", "choose"),
            Node::step("art", "cover-art"),
            Node::step("rip", "rip"),
            branches([
                chain([
                    Node::step("flac", "flac"),
                    Node::step("review", "folder").labelled("Review").with("folder", "~/Music/Review"),
                    Node::step("hold", "hold"),
                    Node::step("lib1", "folder").labelled("Library").with("folder", "~/Music/FLAC"),
                ]),
                chain([Node::step("mp3", "mp3"), Node::step("lib2", "folder").labelled("Library").with("folder", "~/Music/MP3")]),
                Node::step("next", "next-disc"),
            ]),
        ]),
    }
}

/// The most complex recipe we design for: four lookups, two after Choose,
/// four outputs, a branch inside a branch, a named group, and a join at the
/// end.
pub fn archive() -> Recipe {
    Recipe {
        name: "Archive everything".into(),
        root: chain([
            Node::step("a_drive", "drive").with("drive", DVD),
            Node::step("a_cd", "audio-cd"),
            branches([Node::step("a_mb", "musicbrainz"), Node::step("a_ct", "cd-text"), Node::step("a_dg", "discogs"), Node::step("a_gn", "gnudb")]),
            Node::step("a_ch", "choose"),
            branches([Node::step("a_art", "cover-art"), Node::step("a_lyr", "lyrics")]),
            Node::step("a_rip", "rip"),
            branches([
                chain([
                    Node::step("a_flac", "flac"),
                    Node::step("a_rev", "folder").labelled("Review").with("folder", "~/Music/Review"),
                    Node::step("a_hold", "hold"),
                    Node::step("a_lib", "folder").labelled("Library").with("folder", "~/Music/FLAC"),
                    branches([
                        Node::step("a_nas", "folder").labelled("NAS").with("folder", "/mnt/nas/music"),
                        Node::step("a_bak", "folder").labelled("Backup").with("folder", "/run/media/backup"),
                        // Beside the FLACs in the library, from their names.
                        Node::step("a_cue", "cue"),
                        Node::step("a_lrc", "lyrics-file"),
                        Node::step("a_scans", "artwork-files"),
                    ]),
                ]),
                chain([Node::step("a_mp3", "mp3"), Node::step("a_mlib", "folder").labelled("Library").with("folder", "~/Music/MP3")]),
                group("For the phone", chain([Node::step("a_opus", "opus"), Node::step("a_rg", "replaygain"), Node::step("a_ph", "folder").labelled("Phone").with("folder", "~/Sync/Phone")])),
            ]),
            Node::step("a_tell", "tell"),
        ]),
    }
}

/// Data discs: an image of the disc, checked against Redump, made into a
/// CHD and filed in a folder named for it; and, as soon as it's checked,
/// the tray opened for the next.
pub fn data() -> Recipe {
    Recipe {
        name: "Data discs".into(),
        root: chain([
            Node::step("d_drive", "drive").with("drive", BLU_RAY),
            Node::step("d_disc", "data-disc"),
            Node::step("d_image", "image"),
            Node::step("d_check", "check"),
            branches([
                chain([Node::step("d_chd", "chd"), Node::step("d_lib", "folder").labelled("Library").with("folder", "~/Images").with("names", crate::template::DUMP_DEFAULT)]),
                Node::step("d_next", "next-disc"),
            ]),
        ]),
    }
}

/// Games: the dump checked against Redump, named by the entry its hash
/// matches (asking only when it matches nothing), made into a CHD, filed
/// under its system with Redump's LibCrypt sidecar if it needs one, and a
/// multi-disc game's playlist kept beside its discs; and, as soon as it's checked, the tray opened for the next disc.
pub fn games() -> Recipe {
    Recipe {
        name: "Games".into(),
        root: chain([
            Node::step("g_drive", "drive").with("drive", BLU_RAY),
            Node::step("g_disc", "game-disc"),
            Node::step("g_rip", "game-rip"),
            Node::step("g_check", "check"),
            Node::step("g_redump", "redump"),
            Node::step("g_choose", "game-choose"),
            branches([
                chain([
                    Node::step("g_chd", "chd"),
                    Node::step("g_lib", "folder").labelled("Library").with("folder", "~/Games").with("names", "{system}/{name}"),
                    Node::step("g_sbi", "libcrypt"),
                    Node::step("g_m3u", "m3u"),
                ]),
                Node::step("g_next", "next-disc"),
            ]),
        ]),
    }
}

/// Films and TV: the titles listed with MakeMKV, identified with TMDb,
/// matched to the film or to episodes, only those ripped, made smaller and
/// filed for Jellyfin; and, as soon as the last title is read, the tray
/// opened for the next disc.
pub fn video() -> Recipe {
    Recipe {
        name: "Films and TV".into(),
        root: chain([
            Node::step("v_drive", "drive").with("drive", BLU_RAY),
            Node::step("v_disc", "video-disc"),
            Node::step("v_tmdb", "tmdb"),
            Node::step("v_choose", "choose-video"),
            Node::step("v_rip", "rip-video"),
            branches([
                chain([Node::step("v_make", "transcode"), Node::step("v_lib", "folder").labelled("Library").with("folder", "~/Videos").with("names", "Jellyfin")]),
                Node::step("v_next", "next-disc"),
            ]),
        ]),
    }
}

pub fn all() -> Vec<Recipe> {
    vec![cd(), archive(), data(), games(), video()]
}
