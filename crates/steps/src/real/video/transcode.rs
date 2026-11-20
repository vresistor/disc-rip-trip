//! Transcode and Remux: a ripped title made smaller (H.265 or AV1), or kept
//! as it is with only the tracks you want. Settings in the encoders' style
//! (DESIGN-UX.md, "Encoders"), turned into ffmpeg arguments here and
//! nowhere else. Either way the result is checked to be the whole title
//! before it goes on: an encoder exiting 0 on a truncated file is how
//! *Zulu* reached the library with two minutes of picture.
//!
//! Whether to deinterlace isn't a setting: Transcode looks at the title,
//! deinterlaces when it is interlaced, and logs what it decided and why.
//! A stream flagged progressive is taken at its word. Any other flag is
//! checked with ffmpeg's `idet` on a stretch of the picture, because a
//! DVD of a film is often flagged interlaced when its frames aren't, and
//! deinterlacing those would only soften them.

use super::super::tool;
use super::{facts, makemkv};
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use drt_engine::setting::ParamKind;
use drt_engine::{Cx, Group, Inlet, Item, ItemKind, ItemState, LaneKind, Outlet, ParamSpec, Step, StepInfo, Subject};
use serde_json::Value;
use std::path::PathBuf;

const TRACKS: &str = "Tracks";
const ADV: &str = "Advanced";

pub const CODECS: &[&str] = &["H.265", "AV1", "Copy"];
const X265_PRESETS: &[&str] = &["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"];
const AUDIO: &[&str] = &["Copy", "AAC", "Opus"];

type Get<'a> = &'a dyn Fn(&str) -> Option<Value>;

fn s(get: Get, k: &str) -> String {
    match get(k) {
        Some(Value::String(v)) => v,
        Some(v) => v.to_string(),
        None => String::new(),
    }
}

fn n(get: Get, k: &str) -> i64 {
    get(k).and_then(|v| v.as_i64()).unwrap_or(0)
}

fn flag(get: Get, k: &str) -> bool {
    get(k).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn track_params(info: StepInfo) -> StepInfo {
    info.param(
        ParamSpec::new("audio_langs", "Audio languages", ParamKind::Text).default("all").section(TRACKS).help("Language codes such as eng, jpn. Empty or all keeps every track; none keeps none."),
    )
    .param(
        ParamSpec::new("subtitle_langs", "Subtitle languages", ParamKind::Text)
            .default("all")
            .section(TRACKS)
            .help("Language codes such as eng, jpn. Empty or all keeps every track; none keeps none."),
    )
}

pub fn transcode_info() -> StepInfo {
    let info = StepInfo::new("transcode", "Smaller video", Group::Make, ItemKind::Title, ItemKind::VideoFile)
        .from("smaller files, H.265 or AV1")
        .param(
            ParamSpec::new("codec", "Video", ParamKind::Choice(CODECS))
                .default("H.265")
                .labels(&["H.265, plays on most TVs and phones", "AV1, smaller, newer devices only", "As on disc, full size"])
                .on_card_as("Video", &["crf_h265", "crf_av1"], |g| {
                    let codec = g("codec").and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
                    let crf = match codec.as_str() {
                        "H.265" => g("crf_h265"),
                        "AV1" => g("crf_av1"),
                        _ => None,
                    };
                    match crf.and_then(|v| v.as_i64()) {
                        Some(n) => format!("{codec} CRF {n}"),
                        None => codec.to_lowercase(),
                    }
                }),
        )
        // x265 and SVT-AV1 count quality and speed differently.
        .param(ParamSpec::new("crf_h265", "Quality (lower looks better, bigger file)", ParamKind::Range { min: 0, max: 51 }).default(22).when("codec", &["H.265"]))
        .param(ParamSpec::new("crf_av1", "Quality (lower looks better, bigger file)", ParamKind::Range { min: 0, max: 63 }).default(30).when("codec", &["AV1"]))
        .param(ParamSpec::new("preset_h265", "Speed (slower makes smaller files)", ParamKind::Choice(X265_PRESETS)).default("slow").when("codec", &["H.265"]).on_card("Speed"))
        .param(ParamSpec::new("preset_av1", "Speed (lower is slower, smaller)", ParamKind::Range { min: 0, max: 13 }).default(6).when("codec", &["AV1"]).on_card("Speed"))
        .param(ParamSpec::new("ten_bit", "10-bit", ParamKind::Toggle).default(true).when("codec", &["H.265", "AV1"]).help("Smoother skies and shadows. A few older devices can't play it."));
    track_params(info)
        .param(
            ParamSpec::new("audio", "Audio", ParamKind::Choice(AUDIO))
                .default("Copy")
                .section(TRACKS)
                .labels(&["As on disc", "AAC, plays everywhere", "Opus, smaller"])
                .help("Pick AAC if a TV or phone plays the video with no sound."),
        )
        .param(ParamSpec::new("extra", "Extra ffmpeg arguments", ParamKind::Text).default("").section(ADV).help("Given to ffmpeg as written, split at spaces. Leave empty unless you know ffmpeg."))
        .summary(|g| {
            let video = match s(g, "codec").as_str() {
                "AV1" => format!("AV1 CRF {}, preset {}", n(g, "crf_av1"), n(g, "preset_av1")),
                "Copy" => "video as it is".to_owned(),
                _ => format!("H.265 CRF {}, {}", n(g, "crf_h265"), s(g, "preset_h265")),
            };
            format!("{video}{}", langs_note(g))
        })
}

pub fn remux_info() -> StepInfo {
    track_params(StepInfo::new("remux", "Original video", Group::Make, ItemKind::Title, ItemKind::VideoFile).from("the video exactly as on the disc"))
        .summary(|g| format!("video as it is{}", langs_note(g)))
}

/// `; audio eng, jpn` when the languages are narrowed.
fn langs_note(g: Get) -> String {
    let a = s(g, "audio_langs");
    match langs(&a) {
        Some(l) if !l.is_empty() => format!("; audio {}", l.join(", ")),
        _ => String::new(),
    }
}

/// `all`, or left empty → every track; `none` → none; else the
/// languages, in the order given: `eng, jpn`.
pub fn langs(s: &str) -> Option<Vec<String>> {
    let s = s.trim().to_lowercase();
    match s.as_str() {
        "none" => Some(vec![]),
        _ => {
            let l: Vec<String> = s.split([',', ' ']).map(str::trim).filter(|l| !l.is_empty() && *l != "all" && *l != "*").map(str::to_owned).collect();
            // Nothing named (empty, `all`, a stray comma): all of them.
            (!l.is_empty()).then_some(l)
        }
    }
}

/// Which streams go into the file: the first video, then audio and
/// subtitles by language, in the order asked. Only languages the scan says
/// the title has are mapped: ffmpeg refuses a language map that matches
/// nothing, and won't take `?` on one. A list the title has none of keeps
/// all its audio rather than make a film with no sound, and says so;
/// languages the scan didn't give keep everything.
pub fn maps(audio: &str, subs: &str, has_audio: &[String], has_subs: &[String]) -> (Vec<String>, Option<String>) {
    let mut a = vec!["-map".to_owned(), "0:v:0".to_owned()];
    let mut note = None;
    let mut by_lang = |kind: &str, want: Option<Vec<String>>, have: &[String], keep_all_if_none: bool, note: &mut Option<String>| match want {
        None => a.extend(["-map".into(), format!("0:{kind}?")]),
        Some(l) if l.is_empty() => {}
        Some(_) if have.is_empty() => a.extend(["-map".into(), format!("0:{kind}?")]),
        Some(l) => {
            let found: Vec<&String> = l.iter().filter(|x| have.contains(x)).collect();
            if found.is_empty() && keep_all_if_none {
                *note = Some(format!("no {} audio; kept all", l.join(", ")));
                a.extend(["-map".into(), format!("0:{kind}?")]);
            }
            for x in found {
                a.extend(["-map".into(), format!("0:{kind}:m:language:{x}")]);
            }
        }
    };
    by_lang("a", langs(audio), has_audio, true, &mut note);
    by_lang("s", langs(subs), has_subs, false, &mut note);
    (a, note)
}

/// What a title's field order says, from ffprobe's `field_order`: `Some`
/// when it settles the question, `None` when it doesn't (unknown, or not
/// given).
pub fn field_order_says(s: &str) -> Option<bool> {
    match s.trim() {
        "progressive" => Some(false),
        "tt" | "bb" | "tb" | "bt" => Some(true),
        _ => None,
    }
}

/// What ffmpeg's `idet` says, from its *Multi frame detection* line
/// (ffmpeg also prints an empty one for a filter graph it set up and
/// dropped, so the one that counted most): interlaced when more frames
/// are top or bottom field first than progressive. `None` when it decided
/// nothing.
pub fn idet_says(text: &str) -> Option<(bool, u64, u64)> {
    let counts = |line: &str| {
        let num = |k: &str| line.split(k).nth(1).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse::<u64>().ok());
        Some((num("TFF:")? + num("BFF:")?, num("Progressive:")?))
    };
    let (fields, prog) = text.lines().filter(|l| l.contains("Multi frame detection:")).filter_map(counts).max_by_key(|(f, p)| f + p)?;
    (fields + prog > 0).then_some((fields > prog, fields, prog))
}

/// Whether a title is interlaced, and why, in a few words for the log.
async fn interlaced(cx: &Cx, file: &std::path::Path, length: u32) -> (bool, String) {
    let mut probe = tool::command("ffprobe");
    probe.args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=field_order", "-of", "default=nw=1:nk=1"]).arg(file);
    probe.stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let order = probe.output().await.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
    let flagged = field_order_says(&order);
    if flagged == Some(false) {
        return (false, "the stream is flagged progressive".into());
    }
    let flag = if order.is_empty() { "no field order".to_owned() } else { format!("field order {order}") };
    // A stretch from a quarter of the way in, past any studio logos.
    let from = (length / 4).min(600).to_string();
    let mut cmd = tool::command("ffmpeg");
    cmd.args(["-nostdin", "-hide_banner", "-nostats", "-ss", &from, "-i"]).arg(file).args(["-map", "0:v:0", "-vf", "idet", "-frames:v", "600", "-an", "-sn", "-f", "null", "-"]);
    let dir = file.parent().unwrap_or(std::path::Path::new("."));
    match tool::run(cmd, dir, cx.cancelled(), |_| {}).await.ok().and_then(|(text, _)| idet_says(&text)) {
        Some((yes, fields, prog)) => (yes, format!("{flag}, and idet counted {fields} interlaced frames and {prog} progressive")),
        None if flagged == Some(true) => (true, format!("{flag}, and idet couldn't tell")),
        None => (false, format!("{flag}, and idet couldn't tell, so taken as progressive")),
    }
}

/// The codec arguments, from the settings in effect; `deinterlace` when
/// the title was found to be interlaced.
pub fn codec_args(get: Get, deinterlace: bool) -> Vec<String> {
    let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut a = match s(get, "codec").as_str() {
        "AV1" => v(&["-c:v", "libsvtav1", "-crf", &n(get, "crf_av1").to_string(), "-preset", &n(get, "preset_av1").to_string()]),
        "Copy" | "" => v(&["-c:v", "copy"]),
        _ => v(&["-c:v", "libx265", "-crf", &n(get, "crf_h265").to_string(), "-preset", &s(get, "preset_h265"), "-x265-params", "log-level=error"]),
    };
    let encoding = matches!(s(get, "codec").as_str(), "AV1" | "H.265");
    if encoding && flag(get, "ten_bit") {
        a.extend(v(&["-pix_fmt", "yuv420p10le"]));
    }
    if encoding && deinterlace {
        // One frame out for each frame in: the frame rate stays the title's.
        a.extend(v(&["-vf", "bwdif=mode=send_frame"]));
    }
    a.extend(match s(get, "audio").as_str() {
        "AAC" => v(&["-c:a", "aac", "-b:a", "384k"]),
        // Family 1 carries surround as well as stereo.
        "Opus" => v(&["-c:a", "libopus", "-b:a", "256k", "-mapping_family", "1"]),
        _ => v(&["-c:a", "copy"]),
    });
    a.extend(v(&["-c:s", "copy"]));
    a
}

/// Transcode, or Remux (`video: false`), for real.
pub struct Make {
    pub video: bool,
}

#[async_trait]
impl Step for Make {
    fn info(&self) -> StepInfo {
        if self.video {
            transcode_info()
        } else {
            remux_info()
        }
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let mut done = 0;
        let mut in_bytes = 0u64;
        // One at a time: one video encode already uses every core.
        while let Some(title) = inlet.recv().await {
            let total = super::selected(title.context());
            let _core = cx.lane(LaneKind::Encode).await;
            made_says(&cx, in_bytes, title.label.clone());
            cx.item(&title, ItemState::Busy, "0%");
            let started = std::time::Instant::now();
            let r = self
                .make(&cx, &title, |f| {
                    let left = (f > 0.02).then(|| started.elapsed().as_secs_f32() * (1.0 - f) / f);
                    let note = match left {
                        Some(s) if s >= 90.0 => format!("{}%, about {} min left", (f * 100.0) as u32, (s / 60.0).round() as u32),
                        _ => format!("{}%", (f * 100.0) as u32),
                    };
                    cx.item(&title, ItemState::Busy, note);
                    cx.progress(Some((done as f32 + f) / total as f32));
                })
                .await;
            match r {
                Ok((path, note)) => {
                    done += 1;
                    in_bytes += title.path.as_ref().and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len());
                    cx.item(&title, ItemState::Ok, note);
                    outlet.send(title.becomes(ItemKind::VideoFile, Some(path))).await;
                }
                Err(e) if cx.stopped() => return Err(e),
                Err(e) => {
                    cx.item(&title, ItemState::Bad, format!("Couldn't make the file: {e}"));
                    cx.log(format!("{}: {e}", title.label));
                }
            }
            let _ = total;
            made_says(&cx, in_bytes, String::new());
        }
        cx.progress(None);
        Ok(())
    }
}

impl Make {
    async fn make(&self, cx: &Cx, title: &Item, progress: impl Fn(f32)) -> Result<(PathBuf, String)> {
        let part = title.index.ok_or_else(|| anyhow!("not a title"))?;
        let src = title.path.clone().ok_or_else(|| anyhow!("no file"))?;
        let dir = cx.work()?.join(if self.video { "transcode" } else { "remux" });
        std::fs::create_dir_all(&dir)?;
        let out = dir.join(format!("title-{part:02}.mkv"));
        let (length, has_audio, has_subs) = {
            let c = title.context();
            let length = c.get(Subject::Part(part), facts::LENGTH).and_then(|l| l.parse::<u32>().ok()).unwrap_or(0);
            let list = |k| c.get(Subject::Part(part), k).unwrap_or("").split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect::<Vec<_>>();
            (length, list(facts::AUDIO), list(facts::SUBTITLES))
        };
        let get = |k: &str| cx.param(k);
        let (map, note) = maps(&cx.text("audio_langs"), &cx.text("subtitle_langs"), &has_audio, &has_subs);
        let mut cmd = tool::command("ffmpeg");
        cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1", "-y", "-i"]).arg(&src);
        cmd.args(&map).args(["-map_chapters", "0", "-map_metadata", "0"]);
        if self.video {
            let encoding = matches!(cx.text("codec").as_str(), "AV1" | "H.265");
            let deinterlace = if encoding {
                let (yes, why) = interlaced(cx, &src, length).await;
                cx.log(format!("{}: {}, as {why}", title.label, if yes { "interlaced, deinterlacing" } else { "progressive, left as it is" }));
                yes
            } else {
                false
            };
            cmd.args(codec_args(&get, deinterlace));
        } else {
            cmd.args(["-c", "copy"]);
        }
        if let Some(t) = super::display_title(title.context(), part) {
            cmd.arg("-metadata").arg(format!("title={t}"));
        }
        if self.video {
            // Yours, by design: arguments, not a shell line (see Encode's).
            cmd.args(cx.text("extra").split_whitespace());
        }
        cmd.arg(&out);
        let (text, status) = tool::run(cmd, &dir, cx.cancelled(), |l| {
            if let Some(us) = l.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                if length > 0 {
                    progress(((us / 1e6) as f32 / length as f32).clamp(0.0, 1.0));
                }
            }
        })
        .await?;
        if !status.success() {
            bail!("ffmpeg: {}", tool::last_meaningful_line(&text).unwrap_or("failed"));
        }
        makemkv::check_whole(&out, length).await?;
        let size = std::fs::metadata(&out).map(|m| super::super::media::size(m.len())).unwrap_or_default();
        Ok((out, [Some(size), note].into_iter().flatten().collect::<Vec<_>>().join(", ")))
    }
}

/// Transcode's outcome: what it has written (*2.1 GB*) and how small
/// against what came in (*28 %*: whether the settings packed well); with
/// what it's on.
pub fn made_says(cx: &Cx, in_bytes: u64, detail: String) {
    cx.view(|v| {
        let out: u64 = v.record.files.iter().map(|f| f.size).sum();
        v.status = if out > 0 { crate::real::media::size(out) } else { String::new() };
        v.qualifier = if out > 0 && in_bytes > 0 { format!("{} %", (out * 100 / in_bytes).max(1)) } else { String::new() };
        v.detail = detail;
    });
}

/// What it has written so far, for the outcome while it works.
pub fn made_size(cx: &Cx, _in_bytes: u64) -> String {
    let mut s = String::new();
    cx.view(|v| {
        let out: u64 = v.record.files.iter().map(|f| f.size).sum();
        s = if out > 0 { crate::real::media::size(out) } else { String::new() };
    });
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn with(info: &StepInfo, set: &[(&str, Value)]) -> impl Fn(&str) -> Option<Value> {
        let set: HashMap<String, Value> = set.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        let defaults: HashMap<String, Value> = info.params.iter().filter_map(|p| Some((p.key.to_string(), p.default.clone()?))).collect();
        move |k: &str| set.get(k).or(defaults.get(k)).cloned()
    }

    #[test]
    fn h265_by_default_and_av1_with_its_own_scale() {
        let info = transcode_info();
        let g = with(&info, &[]);
        assert_eq!(codec_args(&g, false).join(" "), "-c:v libx265 -crf 22 -preset slow -x265-params log-level=error -pix_fmt yuv420p10le -c:a copy -c:s copy");
        assert_eq!((info.summary.unwrap())(&g), "H.265 CRF 22, slow");
        let g = with(&info, &[("codec", "AV1".into()), ("crf_av1", 35.into()), ("ten_bit", false.into()), ("audio", "Opus".into()), ("audio_langs", "eng, jpn".into())]);
        assert_eq!(codec_args(&g, false).join(" "), "-c:v libsvtav1 -crf 35 -preset 6 -c:a libopus -b:a 256k -mapping_family 1 -c:s copy");
        assert_eq!((info.summary.unwrap())(&g), "AV1 CRF 35, preset 6; audio eng, jpn");
        let crf = |k: &str| info.params.iter().find(|p| p.key == k).unwrap().applies(&g);
        assert!(crf("crf_av1") && !crf("crf_h265"), "each codec shows its own scale");
        let g = with(&info, &[("codec", "Copy".into())]);
        assert_eq!(codec_args(&g, false).join(" "), "-c:v copy -c:a copy -c:s copy", "no pixel format or filter on a copy");
    }

    #[test]
    fn interlacing_is_found_not_set() {
        assert!(!transcode_info().params.iter().any(|p| p.key == "deinterlace"));
        assert_eq!(field_order_says("progressive\n"), Some(false));
        assert_eq!(field_order_says("tt"), Some(true));
        assert_eq!(field_order_says("bb"), Some(true));
        assert_eq!(field_order_says("unknown"), None);
        assert_eq!(field_order_says(""), None);
        let idet = "[Parsed_idet_0 @ 0x1] Repeated Fields: Neither: 600 Top: 0 Bottom: 0\n\
                    [Parsed_idet_0 @ 0x1] Single frame detection: TFF: 380 BFF: 0 Progressive: 150 Undetermined: 70\n\
                    [Parsed_idet_0 @ 0x1] Multi frame detection: TFF: 410 BFF: 0 Progressive: 160 Undetermined: 30\n";
        assert_eq!(idet_says(idet), Some((true, 410, 160)));
        assert_eq!(idet_says("Multi frame detection: TFF: 2 BFF: 1 Progressive: 590 Undetermined: 7"), Some((false, 3, 590)));
        assert_eq!(idet_says("Multi frame detection: TFF: 0 BFF: 0 Progressive: 0 Undetermined: 0"), None);
        assert_eq!(idet_says("nothing"), None);
        // As ffmpeg prints it: padded, and an empty count besides.
        let real = "[Parsed_idet_1 @ 0x55] Multi frame detection: TFF:    61 BFF:     0 Progressive:     0 Undetermined:     0\n\
                    [Parsed_idet_1 @ 0x56] Multi frame detection: TFF:     0 BFF:     0 Progressive:     0 Undetermined:     0\n";
        assert_eq!(idet_says(real), Some((true, 61, 0)));
        let g = with(&transcode_info(), &[]);
        assert!(codec_args(&g, true).join(" ").contains("-vf bwdif"));
        assert!(!codec_args(&with(&transcode_info(), &[("codec", "Copy".into())]), true).join(" ").contains("bwdif"), "never on a copy");
    }

    #[test]
    fn tracks_are_kept_by_language_and_never_all_dropped() {
        let have = vec!["eng".to_owned(), "fra".to_owned()];
        let subs = vec!["eng".to_owned(), "spa".to_owned()];
        assert_eq!(maps("all", "all", &have, &subs).0.join(" "), "-map 0:v:0 -map 0:a? -map 0:s?");
        assert_eq!(maps("eng", "none", &have, &subs).0.join(" "), "-map 0:v:0 -map 0:a:m:language:eng");
        assert_eq!(maps("jpn,fra,eng", "eng deu", &have, &subs).0.join(" "), "-map 0:v:0 -map 0:a:m:language:fra -map 0:a:m:language:eng -map 0:s:m:language:eng", "only what the title has");
        let (m, note) = maps("deu", "deu", &have, &subs);
        assert_eq!(m.join(" "), "-map 0:v:0 -map 0:a?", "no German audio: all of it; no German subtitles: none");
        assert_eq!(note.as_deref(), Some("no deu audio; kept all"));
        assert_eq!(maps("eng", "eng", &[], &[]).0.join(" "), "-map 0:v:0 -map 0:a? -map 0:s?", "languages unknown: everything");
        // Left empty means every track, as all does; only none drops them.
        assert_eq!(maps("", " ", &have, &subs).0.join(" "), "-map 0:v:0 -map 0:a? -map 0:s?");
        assert_eq!(maps("none", "None", &have, &subs).0.join(" "), "-map 0:v:0");
        assert_eq!((remux_info().summary.unwrap())(&with(&remux_info(), &[("audio_langs", "".into())])), "video as it is");
        assert_eq!((remux_info().summary.unwrap())(&with(&remux_info(), &[("audio_langs", "eng".into())])), "video as it is; audio eng");
    }
}
