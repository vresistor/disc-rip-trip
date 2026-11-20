//! Transcode's and Remux's arguments and the whole-title check, run on a
//! real ffmpeg and ffprobe with a made-up MKV (no disc). Ignored by default:
//! it needs ffmpeg with libx265, libsvtav1 and libopus on the PATH.
//!
//! cargo test -p drt-steps --test video_tools -- --ignored

use drt_steps::real::video::{makemkv, transcode};
use serde_json::Value;
use std::path::Path;
use std::process::Command;

fn ffmpeg(dir: &Path, args: &[String]) {
    let out = Command::new("ffmpeg").current_dir(dir).args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"]).args(args).output().unwrap();
    assert!(out.status.success(), "ffmpeg {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr));
}

fn probe(file: &Path) -> (makemkv::Lengths, Vec<String>) {
    let out = Command::new("ffprobe").args(["-v", "error", "-select_streams", "v:0", "-show_entries", "format=duration:stream_tags", "-of", "json", "-i"]).arg(file).output().unwrap();
    let lengths = makemkv::parse_lengths(&String::from_utf8_lossy(&out.stdout)).expect("lengths");
    let out = Command::new("ffprobe").args(["-v", "error", "-show_entries", "stream=codec_type,codec_name:stream_tags=language", "-of", "csv=p=0"]).arg(file).output().unwrap();
    (lengths, String::from_utf8_lossy(&out.stdout).lines().map(str::to_owned).collect())
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
#[ignore]
fn the_arguments_work_on_a_real_ffmpeg_and_a_whole_file_passes_the_check() {
    let dir = std::env::temp_dir().join(format!("drt-video-tools-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    ffmpeg(
        &dir,
        &s(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=20:size=320x240:rate=25",
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:d=20",
            "-f",
            "lavfi",
            "-i",
            "sine=f=660:d=20",
            "-map",
            "0",
            "-map",
            "1",
            "-map",
            "2",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            "-metadata:s:a:0",
            "language=eng",
            "-metadata:s:a:1",
            "language=fra",
            "in.mkv",
        ]),
    );
    let have = s(&["eng", "fra"]);
    let get = |set: Vec<(&'static str, Value)>| {
        let info = transcode::transcode_info();
        move |k: &str| set.iter().find(|(x, _)| *x == k).map(|(_, v)| v.clone()).or_else(|| info.params.iter().find(|p| p.key == k).and_then(|p| p.default.clone()))
    };
    let cases: Vec<(&str, Vec<String>, Vec<String>)> = vec![
        ("remux", transcode::maps("eng", "all", &have, &[]).0, s(&["-c", "copy"])),
        ("h265", transcode::maps("all", "all", &have, &[]).0, transcode::codec_args(&get(vec![]), true)),
        ("av1", transcode::maps("fra", "none", &have, &[]).0, transcode::codec_args(&get(vec![("codec", "AV1".into()), ("audio", "Opus".into())]), false)),
    ];
    for (name, map, codec) in cases {
        let mut args = s(&["-i", "in.mkv"]);
        args.extend(map);
        args.extend(s(&["-map_chapters", "0", "-map_metadata", "0"]));
        args.extend(codec);
        args.push(format!("{name}.mkv"));
        ffmpeg(&dir, &args);
        let (lengths, streams) = probe(&dir.join(format!("{name}.mkv")));
        makemkv::whole(20, lengths).unwrap_or_else(|e| panic!("{name}: {e} ({lengths:?})"));
        assert!(makemkv::whole(40, lengths).is_err(), "{name}: a file half the title's length is refused");
        let audio: Vec<&String> = streams.iter().filter(|l| l.contains("audio")).collect();
        match name {
            "remux" => assert_eq!(audio, [&"aac,audio,eng".to_owned()], "{streams:?}"),
            "h265" => {
                assert_eq!(audio.len(), 2, "{streams:?}");
                assert!(streams[0].starts_with("hevc,video"), "{streams:?}");
            }
            _ => {
                assert_eq!(audio, [&"opus,audio,fra".to_owned()], "{streams:?}");
                assert!(streams[0].starts_with("av1,video"), "{streams:?}");
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
