//! Configuration that knows where it came from.
//!
//! Every parameter of every step is a [`Setting`]. Only values someone or
//! something chose are stored: a default is never written, so a better
//! default in a later version reaches everyone who never changed it.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "how", rename_all = "snake_case")]
pub enum Setting<T> {
    /// Required, and nobody has said yet. Its tile asks before the start.
    Unset,
    /// The step's default. Not stored.
    Default,
    /// Picked by you.
    Chosen { value: T, at: u64 },
    /// Found by the app: measured, or from a list such as AccurateRip's drives.
    Detected { value: T, source: String, at: u64 },
}

impl<T: Clone> Setting<T> {
    pub fn chosen(value: T) -> Self {
        Setting::Chosen { value, at: now() }
    }

    pub fn detected(value: T, source: impl Into<String>) -> Self {
        Setting::Detected { value, source: source.into(), at: now() }
    }

    /// The value in effect, given the step's default.
    pub fn resolve(&self, default: Option<&T>) -> Option<T> {
        match self {
            Setting::Chosen { value, .. } | Setting::Detected { value, .. } => Some(value.clone()),
            Setting::Default => default.cloned(),
            Setting::Unset => None,
        }
    }

    /// One word for where the value came from, shown beside it on its tile.
    pub fn provenance(&self) -> &str {
        match self {
            Setting::Unset => "not set",
            Setting::Default => "default",
            Setting::Chosen { .. } => "yours",
            Setting::Detected { source, .. } => source,
        }
    }
}

/// Where a parameter's value lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// In the recipe: a folder, a bitrate.
    Recipe,
    /// On the drive, shared by every recipe that uses it: the read offset.
    Drive,
    /// The app's, shared by every recipe and every drive: an API key.
    /// Stored in [`Settings::app`] under `<step kind>.<key>`, so two steps'
    /// keys never collide.
    App,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParamKind {
    Text,
    Number,
    Folder,
    Toggle,
    Choice(&'static [&'static str]),
    /// Free text with named presets to pick from (a path template).
    Template(&'static [(&'static str, &'static str)]),
    /// A whole number in a range, shown as a slider with its value.
    Range {
        min: i64,
        max: i64,
    },
}

/// One parameter a step takes. Required when it has no default.
#[derive(Clone, Debug)]
pub struct ParamSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: ParamKind,
    pub default: Option<Value>,
    pub scope: Scope,
    /// Can be looked up rather than asked for (a drive's offset, from
    /// AccurateRip's drive list), with [`crate::Step::find`]. Looked up as
    /// soon as the recipe has what the lookup needs; asked for only if
    /// that fails.
    pub findable: bool,
    /// Which part of the sidebar it's in: "" for the step's own settings,
    /// or a named section such as "Tags and art" or "Advanced".
    pub section: &'static str,
    /// Only applies while another setting has one of these values (a
    /// bitrate only in CBR); hidden otherwise.
    pub when: Option<(&'static str, &'static [&'static str])>,
    /// Only needed when the recipe has one of these steps (a drive's read
    /// offset only matters to an audio Rip, not to imaging a data disc).
    pub only_with: &'static [&'static str],
    /// Shown on the step's card, as label and value; see [`OnCard`].
    pub on_card: Option<OnCard>,
    /// One short sentence, only where the name and choices can't say it
    /// all, shown under the setting and on hover. At most [`HELP_MAX`]
    /// characters.
    pub help: Option<&'static str>,
    /// A choice's values as shown, in the order of [`ParamKind::Choice`]'s
    /// values: empty to show each value as it is. Only the words change;
    /// the value stored and compared stays the one in the kind.
    pub labels: &'static [&'static str],
    /// Makes up a value for a text setting, offered as a *Make one up*
    /// button beside its box (an ntfy topic).
    pub suggest: Option<fn() -> String>,
    /// What the setting offers where it stands in the recipe, from what
    /// arrives there: a Folder's names are a track layout for tracks and a
    /// folder layout for a disc image. Without it, or where nothing is
    /// known, it's [`ParamSpec::default`] and the kind's own choices.
    pub fits: Option<Fits>,
}

/// A setting's default and choices for what arrives at its step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fitted {
    pub default: &'static str,
    /// Its presets, by name, for a [`ParamKind::Template`].
    pub presets: &'static [(&'static str, &'static str)],
    /// What can go in it, and what each one is, for a template.
    pub tokens: &'static [(&'static str, &'static str)],
    /// Stored values that mean "the default" here: what a step saved when
    /// one default served every kind (an image's Folder kept on the music
    /// layout). Read as [`Fitted::default`].
    pub was_default: &'static [&'static str],
}

/// Works out a setting's [`Fitted`] from what arrives at its step.
pub type Fits = fn(&crate::check::Flow) -> Fitted;

/// The longest a setting's help may be, in characters.
pub const HELP_MAX: usize = 110;

/// Settings in words, given how to read each one (*160 kbps VBR*).
pub type Words = fn(&dyn Fn(&str) -> Option<Value>) -> String;

/// How a setting reads on its step's card: a short label (*Level* for
/// *Compression*) and, if the value alone doesn't say it, the value as
/// words, given how to read the step's settings (*160 kbps VBR* from the
/// bitrate and the mode). A card shows at most two.
#[derive(Clone, Copy, Debug)]
pub struct OnCard {
    pub label: &'static str,
    pub value: Option<Words>,
    /// Other settings the value is made from: one of them set by you marks
    /// the row as yours too.
    pub with: &'static [&'static str],
}

impl ParamSpec {
    pub fn new(key: &'static str, label: &'static str, kind: ParamKind) -> Self {
        ParamSpec { key, label, kind, default: None, scope: Scope::Recipe, findable: false, section: "", when: None, only_with: &[], on_card: None, help: None, labels: &[], suggest: None, fits: None }
    }

    /// What it changes for you, in one plain sentence.
    pub fn help(mut self, h: &'static str) -> Self {
        self.help = Some(h);
        self
    }

    /// Offers a made-up value beside the box: *Make one up*.
    pub fn suggest(mut self, f: fn() -> String) -> Self {
        self.suggest = Some(f);
        self
    }

    /// The choices' values as shown, one for each value in order.
    pub fn labels(mut self, l: &'static [&'static str]) -> Self {
        self.labels = l;
        self
    }

    /// The choice's values, or none if it isn't a choice.
    pub fn values(&self) -> &'static [&'static str] {
        match self.kind {
            ParamKind::Choice(v) => v,
            _ => &[],
        }
    }

    /// How a value is shown: its label, else the value itself.
    pub fn choice_label<'a>(&self, value: &'a str) -> &'a str {
        self.values().iter().position(|v| *v == value).and_then(|i| self.labels.get(i).copied()).unwrap_or(value)
    }

    pub fn default(mut self, v: impl Into<Value>) -> Self {
        self.default = Some(v.into());
        self
    }

    /// Stored on the drive and shared by every recipe using it.
    pub fn on_drive(mut self) -> Self {
        self.scope = Scope::Drive;
        self
    }

    /// Stored once for the whole app and shared by every recipe (TMDb's
    /// API key).
    pub fn app_wide(mut self) -> Self {
        self.scope = Scope::App;
        self
    }

    pub fn section(mut self, s: &'static str) -> Self {
        self.section = s;
        self
    }

    /// Applies only while `key` has one of `values`.
    pub fn when(mut self, key: &'static str, values: &'static [&'static str]) -> Self {
        self.when = Some((key, values));
        self
    }

    /// Whether it applies, given how to read the other settings.
    pub fn applies(&self, get: impl Fn(&str) -> Option<Value>) -> bool {
        match self.when {
            None => true,
            Some((key, values)) => get(key).and_then(|v| v.as_str().map(str::to_owned)).is_some_and(|v| values.contains(&v.as_str())),
        }
    }

    /// Only needed when the recipe has one of these step kinds.
    pub fn only_with(mut self, kinds: &'static [&'static str]) -> Self {
        self.only_with = kinds;
        self
    }

    /// Shown on the card under `label`, its value as it is.
    pub fn on_card(mut self, label: &'static str) -> Self {
        self.on_card = Some(OnCard { label, value: None, with: &[] });
        self
    }

    /// Shown on the card under `label`, its value as `value` words it,
    /// from this setting and the ones in `with`.
    pub fn on_card_as(mut self, label: &'static str, with: &'static [&'static str], value: Words) -> Self {
        self.on_card = Some(OnCard { label, value: Some(value), with });
        self
    }

    /// Its default and choices follow what arrives at its step.
    pub fn fits(mut self, f: Fits) -> Self {
        self.fits = Some(f);
        self
    }

    /// Its default and choices given what arrives (`None` where that isn't
    /// known: the step's own).
    pub fn fitted(&self, flow: Option<&crate::check::Flow>) -> Option<Fitted> {
        Some((self.fits?)(flow?))
    }

    /// The default where it stands, given what arrives.
    pub fn default_at(&self, flow: Option<&crate::check::Flow>) -> Option<Value> {
        self.fitted(flow).map(|f| Value::from(f.default)).or_else(|| self.default.clone())
    }

    /// The value in effect where it stands: the setting's own, else the
    /// default there; a value that only ever meant the default reads as it.
    pub fn resolve_at(&self, setting: &Setting<Value>, flow: Option<&crate::check::Flow>) -> Option<Value> {
        let default = self.default_at(flow);
        let value = setting.resolve(default.as_ref())?;
        match self.fitted(flow) {
            Some(f) if value.as_str().is_some_and(|v| v.is_empty() || f.was_default.contains(&v)) => default,
            _ => Some(value),
        }
    }

    pub fn findable(mut self) -> Self {
        self.findable = true;
        self
    }

    /// Has to be known before the recipe can start.
    pub fn required(&self) -> bool {
        self.default.is_none()
    }
}

/// Settings that belong to things rather than to recipes: by drive, and
/// the app's own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub drives: BTreeMap<String, BTreeMap<String, Setting<Value>>>,
    /// App-wide settings, keyed `<step kind>.<key>` (`tmdb.api_key`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub app: BTreeMap<String, Setting<Value>>,
}

impl Settings {
    /// An app-wide setting of the step `kind`.
    pub fn app(&self, kind: &str, key: &str) -> Setting<Value> {
        self.app.get(&format!("{kind}.{key}")).cloned().unwrap_or(Setting::Default)
    }

    pub fn set_app(&mut self, kind: &str, key: &str, value: Setting<Value>) {
        self.app.insert(format!("{kind}.{key}"), value);
    }

    /// Where one of a step's settings lives, given the recipe node's own
    /// settings and the recipe's drive.
    pub fn lookup(&self, spec: &ParamSpec, kind: &str, params: &BTreeMap<String, Setting<Value>>, drive: &str) -> Setting<Value> {
        match spec.scope {
            Scope::Recipe => params.get(spec.key).cloned().unwrap_or(Setting::Default),
            Scope::Drive => self.drive(drive, spec.key),
            Scope::App => self.app(kind, spec.key),
        }
    }

    pub fn drive(&self, drive: &str, key: &str) -> Setting<Value> {
        self.drives.get(drive).and_then(|m| m.get(key)).cloned().unwrap_or(Setting::Default)
    }

    pub fn set_drive(&mut self, drive: &str, key: &str, value: Setting<Value>) {
        self.drives.entry(drive.into()).or_default().insert(key.into(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_choice_shows_its_label_and_keeps_its_value() {
        let p = ParamSpec::new("id3", "Tag version", ParamKind::Choice(&["v2.3", "v2.4"])).labels(&["2.3, works with older players", "2.4, newer"]);
        assert_eq!(p.choice_label("v2.3"), "2.3, works with older players");
        // A value from elsewhere (an old file, typed in) shows as it is.
        assert_eq!(p.choice_label("v1"), "v1");
        let plain = ParamSpec::new("x", "X", ParamKind::Choice(&["a"]));
        assert_eq!(plain.choice_label("a"), "a");
    }

    #[test]
    fn defaults_are_never_stored() {
        let mut s = Settings::default();
        s.set_drive("Blu-ray drive", "offset", Setting::detected(Value::from(103), "AccurateRip"));
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("detected"));
        assert_eq!(s.drive("DVD drive", "offset"), Setting::Default);
        assert_eq!(s.drive("Blu-ray drive", "offset").resolve(None), Some(Value::from(103)));
    }

    #[test]
    fn app_settings_are_keyed_by_step_and_load_from_files_without_them() {
        let mut s: Settings = serde_json::from_str(r#"{"drives":{}}"#).unwrap();
        assert_eq!(s.app("tmdb", "api_key"), Setting::Default);
        s.set_app("tmdb", "api_key", Setting::chosen(Value::from("k")));
        assert_eq!(s.app("tmdb", "api_key").resolve(None), Some(Value::from("k")));
        assert_eq!(s.app("other", "api_key"), Setting::Default);
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        let spec = ParamSpec::new("api_key", "API key", ParamKind::Text).app_wide();
        assert_eq!(s.lookup(&spec, "tmdb", &BTreeMap::new(), "any drive").resolve(None), Some(Value::from("k")));
    }
}
