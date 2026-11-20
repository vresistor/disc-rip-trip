//! Items: what travels between steps.

use crate::context::Context;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// The kind of item a step takes or gives. Checked whenever a recipe is
/// loaded or edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// What the first step of a recipe takes, and what a last step like
    /// *Notify* gives.
    Nothing,
    /// A disc nothing has looked at yet: what a Drive gives. A kind step
    /// says what it is.
    Disc,
    AudioDisc,
    DataDisc,
    GameDisc,
    VideoDisc,
    Track,
    Title,
    Dump,
    /// An encoded track (FLAC, MP3…).
    AudioFile,
    /// A disc image made from a dump (CHD, or the dump kept as it is).
    ImageFile,
    /// A film or episode made from a title (MKV).
    VideoFile,
    /// A text file made beside the rest (a cue sheet).
    TextFile,
    /// Taken by a step that takes any file (Folder), and what branches
    /// giving different kinds of file join into. Never an item's own kind.
    /// Recipes and snapshots written when every file was one kind, `file`,
    /// read it as this.
    #[serde(alias = "file")]
    AnyFile,
    /// Taken by a step that accepts anything but nothing (Hold, Notify),
    /// and what branches giving different kinds join into.
    Any,
}

impl ItemKind {
    /// Whether a step taking `self` can be fed `given`.
    /// `Any` takes everything except `Nothing` (what follows Notify or
    /// Next disc); `AnyFile` takes every kind of file.
    pub fn accepts(self, given: ItemKind) -> bool {
        match self {
            ItemKind::Any => given != ItemKind::Nothing,
            ItemKind::AnyFile => given.is_file(),
            _ => self == given,
        }
    }

    /// Any file, of whatever kind.
    pub fn is_file(self) -> bool {
        matches!(self, ItemKind::AudioFile | ItemKind::ImageFile | ItemKind::VideoFile | ItemKind::TextFile | ItemKind::AnyFile)
    }

    /// Any disc, whether or not a kind step has said what it is.
    pub fn is_disc(self) -> bool {
        matches!(self, ItemKind::Disc | ItemKind::AudioDisc | ItemKind::DataDisc | ItemKind::GameDisc | ItemKind::VideoDisc)
    }
}

/// One unit of work. Items are identified by `id` across the whole run, so a
/// join can tell that a FLAC file and an MP3 file are the same track.
///
/// An item carries what's known about it: the steps it has passed added to
/// its context, and that is all a step after them can know. Shared between
/// copies until one of them learns something.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub kind: ItemKind,
    pub label: String,
    /// The track or title number, when there is one.
    pub index: Option<u32>,
    /// The file this item is, once it is one.
    pub path: Option<PathBuf>,
    pub context: Arc<Context>,
}

impl Item {
    /// A new item that knows nothing yet.
    pub fn new(id: impl Into<String>, kind: ItemKind, label: impl Into<String>, index: Option<u32>, path: Option<PathBuf>) -> Self {
        Item { id: id.into(), kind, label: label.into(), index, path, context: Arc::default() }
    }

    pub fn disc(label: impl Into<String>) -> Self {
        Item::new("disc", ItemKind::Disc, label, None, None)
    }

    pub fn track(n: u32, label: impl Into<String>) -> Self {
        Item::new(format!("track-{n}"), ItemKind::Track, label, Some(n), None)
    }

    /// The same item, now of another kind (a track that has become a file).
    pub fn becomes(&self, kind: ItemKind, path: Option<PathBuf>) -> Self {
        Item { kind, path, ..self.clone() }
    }

    /// This new item knows what `from` knew: the tracks a disc becomes,
    /// a cue sheet made of tracks.
    pub fn knowing(mut self, from: &Item) -> Self {
        self.context = from.context.clone();
        self
    }

    /// What's known about it.
    pub fn context(&self) -> &Context {
        &self.context
    }

    /// To add to what's known about it.
    pub fn context_mut(&mut self) -> &mut Context {
        Arc::make_mut(&mut self.context)
    }
}

#[cfg(test)]
mod tests {
    use super::ItemKind;

    #[test]
    fn the_old_single_file_kind_still_reads() {
        assert_eq!(serde_json::from_str::<ItemKind>("\"file\"").unwrap(), ItemKind::AnyFile);
        for k in [ItemKind::AudioFile, ItemKind::ImageFile, ItemKind::VideoFile, ItemKind::TextFile, ItemKind::AnyFile] {
            assert_eq!(serde_json::from_str::<ItemKind>(&serde_json::to_string(&k).unwrap()).unwrap(), k);
        }
        assert_eq!(serde_json::to_string(&ItemKind::AudioFile).unwrap(), "\"audio_file\"");
    }

    #[test]
    fn what_takes_what() {
        assert!(ItemKind::AnyFile.accepts(ItemKind::ImageFile));
        assert!(!ItemKind::AnyFile.accepts(ItemKind::Track));
        assert!(!ItemKind::AudioFile.accepts(ItemKind::ImageFile));
        assert!(ItemKind::Any.accepts(ItemKind::AudioFile));
        assert!(!ItemKind::Any.accepts(ItemKind::Nothing), "nothing follows what gives nothing");
    }
}
