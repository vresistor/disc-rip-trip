//! `drt fix-tags`: mends the two tagging mistakes in files filed before
//! the audio audit of 20 November 2026, and nothing else.
//!
//! 1. cyanrip put its name in every WAV it wrote (`comment=cyanrip
//!    0.9.3`), and ffmpeg copied it into every file: a Vorbis
//!    `DESCRIPTION`, an MP3 `COMM` frame, an MP4 `©cmt` atom, an APE
//!    `comment`, a RIFF `ICMT`. A comment whose whole value is exactly
//!    `cyanrip <version>` is removed; any other comment stays.
//! 2. The cover went in as picture type 0, *Other*, not 3, *Front cover*
//!    (FLAC and MP3). It's changed only in a file that also carries the
//!    signature, so this app made it, and that holds exactly one picture.
//!
//! Tags only, never audio. Each file is changed as a copy beside it; the
//! copy is checked against the original and only then renamed over it,
//! so a check that fails leaves the original as it was. The checks: the
//! decoded audio's MD5 is the same; every tag ffprobe sees is the same but
//! for the change; every tag lofty reads (frames and atoms ffprobe doesn't
//! show, pictures by their bytes) is the same but for the change.

use super::{encode, tool};
use anyhow::{anyhow, bail, Result};
use lofty::config::{ParseOptions, WriteOptions};
use lofty::ogg::OggPictureStorage;
use lofty::picture::PictureType;
use lofty::prelude::*;
use std::path::{Path, PathBuf};

/// cyanrip's signature: `cyanrip 0.9.3`, as its source sets it
/// (`"cyanrip " PROJECT_VERSION_STRING`), and nothing more.
pub fn is_signature(v: &str) -> bool {
    v.strip_prefix("cyanrip ").is_some_and(|ver| ver.starts_with(|c: char| c.is_ascii_digit()) && ver.chars().all(|c| c.is_ascii_digit() || c == '.'))
}

/// What would change in one file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub signature: bool,
    pub front_cover: bool,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        !self.signature && !self.front_cover
    }

    /// *remove the cyanrip comment; mark the picture as the front cover*.
    pub fn describe(&self) -> String {
        let mut d = vec![];
        if self.signature {
            d.push("remove the cyanrip comment");
        }
        if self.front_cover {
            d.push("mark the picture as the front cover");
        }
        d.join("; ")
    }
}

/// The audio files under `dir`, by name, that this knows how to read.
pub fn files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = vec![];
    let mut todo = vec![dir.to_owned()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).map_err(|e| anyhow!("{}: {e}", d.display()))? {
            let p = e?.path();
            if p.is_dir() {
                todo.push(p);
            } else if kind(&p).is_some() {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Flac,
    Mp3,
    Opus,
    Vorbis,
    Mp4,
    WavPack,
    Wav,
}

fn kind(p: &Path) -> Option<Kind> {
    match p.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "flac" => Some(Kind::Flac),
        "mp3" => Some(Kind::Mp3),
        "opus" => Some(Kind::Opus),
        "ogg" | "oga" => Some(Kind::Vorbis),
        "m4a" | "mp4" => Some(Kind::Mp4),
        "wv" => Some(Kind::WavPack),
        "wav" => Some(Kind::Wav),
        _ => None,
    }
}

fn parse() -> ParseOptions {
    ParseOptions::new().implicit_conversions(false)
}

/// A file read with lofty, its tag in the form the container has it.
enum Tags {
    Flac(lofty::flac::FlacFile),
    Mp3(lofty::mpeg::MpegFile),
    Opus(lofty::ogg::OpusFile),
    Vorbis(lofty::ogg::VorbisFile),
    Mp4(lofty::mp4::Mp4File),
    WavPack(lofty::wavpack::WavPackFile),
    Wav(lofty::iff::wav::WavFile),
}

fn read(p: &Path) -> Result<Tags> {
    let k = kind(p).ok_or_else(|| anyhow!("not an audio file"))?;
    let mut f = std::fs::File::open(p)?;
    let o = parse();
    Ok(match k {
        Kind::Flac => Tags::Flac(lofty::flac::FlacFile::read_from(&mut f, o)?),
        Kind::Mp3 => Tags::Mp3(lofty::mpeg::MpegFile::read_from(&mut f, o)?),
        Kind::Opus => Tags::Opus(lofty::ogg::OpusFile::read_from(&mut f, o)?),
        Kind::Vorbis => Tags::Vorbis(lofty::ogg::VorbisFile::read_from(&mut f, o)?),
        Kind::Mp4 => Tags::Mp4(lofty::mp4::Mp4File::read_from(&mut f, o)?),
        Kind::WavPack => Tags::WavPack(lofty::wavpack::WavPackFile::read_from(&mut f, o)?),
        Kind::Wav => Tags::Wav(lofty::iff::wav::WavFile::read_from(&mut f, o)?),
    })
}

/// A picture as one line: its type and what it is, by its bytes.
fn picture_line(p: &lofty::picture::Picture) -> String {
    format!("picture\t{:?}\t{:?}\t{:?}\t{}\t{:08x}", p.pic_type(), p.mime_type(), p.description(), p.data().len(), crc32fast::hash(p.data()))
}

const COMMENT: &str = "comment\t";

/// Every tag in the file as lines, in a form where the two changes are
/// easy to state: the signature is `comment\t<value>`, a picture
/// `picture\t<type>\t…`. Anything else as lofty has it.
fn inventory(t: &Tags) -> Vec<String> {
    let mut out = vec![];
    let vorbis = |vc: &lofty::ogg::tag::VorbisComments, out: &mut Vec<String>| {
        out.push(format!("vendor\t{}", vc.vendor()));
        for (k, v) in vc.items() {
            if (k.eq_ignore_ascii_case("description") || k.eq_ignore_ascii_case("comment")) && is_signature(v) {
                out.push(format!("{COMMENT}{v}"));
            } else {
                out.push(format!("item\t{k}\t{v}"));
            }
        }
    };
    match t {
        Tags::Flac(f) => {
            if let Some(vc) = f.vorbis_comments() {
                vorbis(vc, &mut out);
            }
            for (p, i) in f.pictures() {
                out.push(format!("{}\t{:?}", picture_line(p), i));
            }
        }
        Tags::Opus(f) => {
            vorbis(f.vorbis_comments(), &mut out);
            out.extend(f.vorbis_comments().pictures().iter().map(|(p, i)| format!("{}\t{:?}", picture_line(p), i)));
        }
        Tags::Vorbis(f) => {
            vorbis(f.vorbis_comments(), &mut out);
            out.extend(f.vorbis_comments().pictures().iter().map(|(p, i)| format!("{}\t{:?}", picture_line(p), i)));
        }
        Tags::Mp3(f) => {
            if let Some(t) = f.id3v2() {
                out.push(format!("id3\t{:?}\t{:?}", t.original_version(), t.flags()));
                for fr in t {
                    match fr {
                        lofty::id3::v2::Frame::Comment(c) if is_signature(&c.content) => out.push(format!("{COMMENT}{}", c.content)),
                        // Some ffmpeg builds wrote it as TXXX:comment.
                        lofty::id3::v2::Frame::UserText(x) if x.description.eq_ignore_ascii_case("comment") && is_signature(&x.content) => out.push(format!("{COMMENT}{}", x.content)),
                        lofty::id3::v2::Frame::Picture(p) => out.push(format!("{}\t{:?}", picture_line(&p.picture), p.encoding)),
                        other => out.push(format!("frame\t{other:?}")),
                    }
                }
            }
            if let Some(v1) = f.id3v1() {
                out.push(format!("id3v1\t{v1:?}"));
            }
            if let Some(a) = f.ape() {
                out.push(format!("ape\t{a:?}"));
            }
        }
        Tags::Mp4(f) => {
            if let Some(ilst) = f.ilst() {
                for a in ilst {
                    let cmt = matches!(a.ident(), lofty::mp4::AtomIdent::Fourcc(b) if b == b"\xa9cmt");
                    let texts: Vec<&str> = a.data().filter_map(|d| if let lofty::mp4::AtomData::UTF8(s) = d { Some(s.as_str()) } else { None }).collect();
                    if cmt && texts.len() == 1 && is_signature(texts[0]) {
                        out.push(format!("{COMMENT}{}", texts[0]));
                    } else if matches!(a.ident(), lofty::mp4::AtomIdent::Fourcc(b) if b == b"covr") {
                        for d in a.data() {
                            // MP4 has no picture types: never "Other" to change.
                            if let lofty::mp4::AtomData::Picture(p) = d {
                                out.push(format!("mp4 {}", picture_line(p)));
                            } else {
                                out.push(format!("atom\tcovr\t{d:?}"));
                            }
                        }
                    } else {
                        out.push(format!("atom\t{a:?}"));
                    }
                }
            }
        }
        Tags::WavPack(f) => {
            if let Some(ape) = f.ape() {
                for i in ape {
                    match i.value() {
                        lofty::tag::ItemValue::Text(v) if i.key().eq_ignore_ascii_case("comment") && is_signature(v) => out.push(format!("{COMMENT}{v}")),
                        v => out.push(format!("ape\t{}\t{v:?}", i.key())),
                    }
                }
            }
            if let Some(v1) = f.id3v1() {
                out.push(format!("id3v1\t{v1:?}"));
            }
        }
        Tags::Wav(f) => {
            if let Some(info) = f.riff_info() {
                for (k, v) in info {
                    if k == "ICMT" && is_signature(v) {
                        out.push(format!("{COMMENT}{v}"));
                    } else {
                        out.push(format!("info\t{k}\t{v}"));
                    }
                }
            }
            if let Some(id3) = f.id3v2() {
                out.push(format!("id3\t{id3:?}"));
            }
        }
    }
    out.sort();
    out
}

/// What the inventory should be after `plan`.
fn expected(before: &[String], plan: &Plan) -> Vec<String> {
    let mut out: Vec<String> = before
        .iter()
        .filter(|l| !(plan.signature && l.starts_with(COMMENT)))
        .map(|l| if plan.front_cover && l.starts_with("picture\tOther\t") { l.replacen("picture\tOther\t", "picture\tCoverFront\t", 1) } else { l.clone() })
        .collect();
    out.sort();
    out
}

fn plan_of(inv: &[String]) -> Plan {
    let signature = inv.iter().any(|l| l.starts_with(COMMENT));
    let pictures: Vec<&String> = inv.iter().filter(|l| l.starts_with("picture\t")).collect();
    Plan { signature, front_cover: signature && pictures.len() == 1 && pictures[0].starts_with("picture\tOther\t") }
}

/// What `fix-tags` would change in `file`.
pub fn plan(file: &Path) -> Result<Plan> {
    Ok(plan_of(&inventory(&read(file)?)))
}

/// Makes the changes in `t`, in memory.
fn change(t: &mut Tags, plan: &Plan) {
    let fix_pic = |p: &mut lofty::picture::Picture| {
        if p.pic_type() == PictureType::Other {
            p.set_pic_type(PictureType::CoverFront);
        }
    };
    let drop_sig = |vc: &mut lofty::ogg::tag::VorbisComments| {
        for k in ["DESCRIPTION", "COMMENT"] {
            let keep: Vec<String> = vc.get_all(k).filter(|v| !is_signature(v)).map(str::to_owned).collect();
            if vc.get_all(k).any(is_signature) {
                let _ = vc.remove(k).count();
                for v in keep {
                    vc.push(k.to_owned(), v);
                }
            }
        }
    };
    match t {
        Tags::Flac(f) => {
            if plan.signature {
                if let Some(vc) = f.vorbis_comments_mut() {
                    drop_sig(vc);
                }
            }
            if plan.front_cover {
                let (mut p, i) = f.pictures()[0].clone();
                fix_pic(&mut p);
                f.set_picture(0, p, i);
            }
        }
        Tags::Opus(_) | Tags::Vorbis(_) => {
            let vc = match t {
                Tags::Opus(f) => f.vorbis_comments_mut(),
                Tags::Vorbis(f) => f.vorbis_comments_mut(),
                _ => unreachable!(),
            };
            if plan.signature {
                drop_sig(vc);
            }
            if plan.front_cover {
                let (mut p, i) = vc.pictures()[0].clone();
                fix_pic(&mut p);
                vc.set_picture(0, p, i);
            }
        }
        Tags::Mp3(f) => {
            if let Some(tag) = f.id3v2_mut() {
                let pic = (&*tag).into_iter().find_map(|fr| if let lofty::id3::v2::Frame::Picture(p) = fr { Some(p.clone()) } else { None });
                tag.retain(|fr| match fr {
                    lofty::id3::v2::Frame::Comment(c) => !(plan.signature && is_signature(&c.content)),
                    lofty::id3::v2::Frame::UserText(x) => !(plan.signature && x.description.eq_ignore_ascii_case("comment") && is_signature(&x.content)),
                    lofty::id3::v2::Frame::Picture(_) => !plan.front_cover,
                    _ => true,
                });
                if let (true, Some(p)) = (plan.front_cover, pic) {
                    let mut picture = p.picture.into_owned();
                    fix_pic(&mut picture);
                    tag.insert(lofty::id3::v2::Frame::Picture(lofty::id3::v2::AttachedPictureFrame::new(p.encoding, picture)));
                }
            }
        }
        Tags::Mp4(f) => {
            if let Some(ilst) = f.ilst_mut() {
                ilst.retain(|a| {
                    let cmt = matches!(a.ident(), lofty::mp4::AtomIdent::Fourcc(b) if b == b"\xa9cmt");
                    let texts: Vec<&str> = a.data().filter_map(|d| if let lofty::mp4::AtomData::UTF8(s) = d { Some(s.as_str()) } else { None }).collect();
                    !(cmt && texts.len() == 1 && is_signature(texts[0]))
                });
            }
        }
        Tags::WavPack(f) => {
            if let Some(ape) = f.ape_mut() {
                let key = (&*ape).into_iter().find(|i| i.key().eq_ignore_ascii_case("comment") && matches!(i.value(), lofty::tag::ItemValue::Text(v) if is_signature(v))).map(|i| i.key().to_owned());
                if let Some(k) = key {
                    ape.remove(&k);
                }
            }
        }
        Tags::Wav(f) => {
            if let Some(info) = f.riff_info_mut() {
                if info.get("ICMT").is_some_and(is_signature) {
                    info.remove("ICMT");
                }
            }
        }
    }
}

fn save(t: &Tags, p: &Path) -> Result<()> {
    // A write reads the file again to find the tag; it reads it as `parse` does.
    let mut w = WriteOptions::default().preferred_padding(0).parse_options(parse());
    match t {
        Tags::Flac(f) => f.save_to_path(p, w)?,
        Tags::Mp3(f) => {
            let tag = f.id3v2().ok_or_else(|| anyhow!("no ID3v2 tag"))?;
            let v23 = tag.original_version() == lofty::id3::v2::Id3v2Version::V3;
            tag.save_to_path(p, w.use_id3v23(v23))?
        }
        Tags::Opus(f) => f.vorbis_comments().save_to_path(p, w)?,
        Tags::Vorbis(f) => f.vorbis_comments().save_to_path(p, w)?,
        Tags::Mp4(f) => f.ilst().ok_or_else(|| anyhow!("no tags"))?.save_to_path(p, w)?,
        Tags::WavPack(f) => f.ape().ok_or_else(|| anyhow!("no APE tag"))?.save_to_path(p, w)?,
        Tags::Wav(f) => f.riff_info().ok_or_else(|| anyhow!("no INFO"))?.save_to_path(p, w)?,
    }
    Ok(())
}

/// Every tag ffprobe sees, and each stream's kind and disposition, sorted.
async fn probed(file: &Path) -> Result<Vec<String>> {
    let mut cmd = tool::command("ffprobe");
    cmd.args(["-v", "error", "-show_entries", "stream=index,codec_type,codec_name:stream_disposition:stream_tags:format_tags", "-of", "compact=nk=0", "-i"]).arg(file);
    let (text, status) = tool::run(cmd, file.parent().unwrap_or(Path::new(".")), std::future::pending(), |_| {}).await?;
    if !status.success() {
        bail!("ffprobe couldn't read {}: {}", file.display(), tool::last_meaningful_line(&text).unwrap_or("no output"));
    }
    let mut out = vec![];
    for line in text.lines().filter(|l| l.starts_with("stream|") || l.starts_with("format|")) {
        let mut fields: Vec<&str> = line.split('|').collect();
        let section = fields.remove(0);
        for f in fields {
            out.push(format!("{section}|{f}"));
        }
    }
    out.sort();
    Ok(out)
}

fn probed_expected(before: &[String], plan: &Plan) -> Vec<String> {
    let mut out: Vec<String> = before
        .iter()
        .filter(|l| !(plan.signature && l.split_once("|tag:comment=").is_some_and(|(_, v)| is_signature(v))))
        .map(|l| if plan.front_cover && l == "stream|tag:comment=Other" { "stream|tag:comment=Cover (front)".to_owned() } else { l.clone() })
        .collect();
    out.sort();
    out
}

/// Makes `plan`'s changes to `file`: on a copy, checked, then renamed over
/// it. Gives an error, and leaves the file as it was, if any check fails.
pub async fn apply(file: &Path, plan: &Plan) -> Result<()> {
    if plan.is_empty() {
        return Ok(());
    }
    // The tools run in the file's folder: a relative path would point
    // somewhere else from there.
    let file = &std::fs::canonicalize(file)?;
    let before = read(file)?;
    let inv = inventory(&before);
    if plan_of(&inv) != *plan {
        bail!("the file changed since it was looked at");
    }
    let (probe, audio) = (probed(file).await?, encode::pcm_md5(file, std::future::pending()).await?);
    // Beside it, so the rename is atomic; with its extension, so it's read
    // as the same kind of file.
    let stem = file.file_stem().ok_or_else(|| anyhow!("no file name"))?.to_string_lossy().into_owned();
    let ext = file.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let copy = file.with_file_name(format!(".{stem}.fix-tags.{ext}"));
    std::fs::copy(file, &copy)?;
    let checked: Result<()> = async {
        let mut t = read(&copy)?;
        change(&mut t, plan);
        save(&t, &copy)?;
        let after = inventory(&read(&copy)?);
        if after != expected(&inv, plan) {
            let gone: Vec<&String> = inv.iter().filter(|l| !after.contains(l)).collect();
            let new: Vec<&String> = after.iter().filter(|l| !inv.contains(l)).collect();
            bail!("tags other than these would change (before {gone:?}, after {new:?})");
        }
        if probed(&copy).await? != probed_expected(&probe, plan) {
            bail!("ffprobe sees other tags change");
        }
        let now = encode::pcm_md5(&copy, std::future::pending()).await?;
        if now != audio {
            bail!("the audio would change ({audio} ≠ {now})");
        }
        Ok(())
    }
    .await;
    match checked {
        Ok(()) => {
            std::fs::rename(&copy, file)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&copy);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_cyanrips_own_words_are_its_signature() {
        assert!(is_signature("cyanrip 0.9.3"));
        assert!(is_signature("cyanrip 1.0"));
        assert!(!is_signature("cyanrip 0.9.3 (6f7f238)"));
        assert!(!is_signature("ripped with cyanrip 0.9.3"));
        assert!(!is_signature("cyanrip"));
        assert!(!is_signature("cyanrip is great"));
    }

    #[test]
    fn the_cover_is_changed_only_beside_the_signature_and_alone() {
        let pic = "picture\tOther\tSome(Jpeg)\tNone\t10\t0".to_owned();
        let sig = format!("{COMMENT}cyanrip 0.9.3");
        assert_eq!(plan_of(&[sig.clone(), pic.clone()]), Plan { signature: true, front_cover: true });
        assert_eq!(plan_of(std::slice::from_ref(&pic)), Plan::default(), "not ours: left alone");
        assert_eq!(plan_of(&[sig.clone(), pic.clone(), pic.replace("10", "11")]), Plan { signature: true, front_cover: false }, "two pictures");
        assert_eq!(plan_of(&[sig.clone(), pic.replace("Other", "CoverFront")]), Plan { signature: true, front_cover: false });
        let after = expected(&[sig, pic, "item\tTITLE\tX".into()], &Plan { signature: true, front_cover: true });
        assert_eq!(after, ["item\tTITLE\tX", "picture\tCoverFront\tSome(Jpeg)\tNone\t10\t0"]);
        assert_eq!(Plan { signature: true, front_cover: true }.describe(), "remove the cyanrip comment; mark the picture as the front cover");
    }
}
