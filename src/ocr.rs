//! Text recognition, like CleanShot's "Capture Text": pick an area (or use
//! a screenshot you already have), the text goes to the clipboard and a
//! card like a screenshot's shows what was found. Tesseract does the reading, with
//! every installed language that matters here.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use gpui::{App, AsyncApp};

use crate::kit;
use crate::util;
use crate::i18n::t;

/// `kadr text`: select an area on a frozen screen and read it.
pub fn region(cx: &mut App) {
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let _hold = hold;
        let Some(png) = grab_region(cx).await else { return };
        let text = recognize(&png, Layout::Block).await;
        let _ = std::fs::remove_file(&png);
        let _ = cx.update(|cx| done(text, cx));
    })
    .detach();
}

/// Read a whole picture (a screenshot card, the editor).
pub fn image(path: PathBuf, cx: &mut App) {
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let _hold = hold;
        let text = recognize(&path, Layout::Auto).await;
        let _ = cx.update(|cx| done(text, cx));
    })
    .detach();
}

/// kadr's picker over the frozen screen; the picked part as a PNG.
async fn grab_region(cx: &mut AsyncApp) -> Option<PathBuf> {
    let picked = crate::picker::pick(false, true, cx).await?;
    let png = util::runtime_dir().join(format!("kadr-ocr-{}.png", std::process::id()));
    picked.crop().save(&png).ok()?;
    Some(png)
}

enum Layout {
    /// A selected block of text.
    Block,
    /// A full screenshot with columns, sidebars and so on.
    Auto,
}

/// kadr's own language models (system ones linked in), for languages the
/// distro packages don't have installed; used instead of the system folder
/// when present.
pub(crate) fn tessdata_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("kadr/tessdata")).filter(|d| d.is_dir())
}

/// Installed Tesseract languages kadr cares about, e.g. "rus+eng".
pub(crate) fn languages() -> String {
    let mut cmd = std::process::Command::new("tesseract");
    if let Some(dir) = tessdata_dir() {
        cmd.arg("--tessdata-dir").arg(dir);
    }
    let installed = cmd
        .arg("--list-langs")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let wanted: Vec<&str> = ["rus", "eng", "ukr", "deu", "fra", "spa"]
        .into_iter()
        .filter(|l| installed.lines().any(|i| i.trim() == *l))
        .collect();
    if wanted.is_empty() { "eng".into() } else { wanted.join("+") }
}

pub fn has_russian() -> bool {
    languages().split('+').any(|l| l == "rus")
}

async fn recognize(png: &Path, layout: Layout) -> Option<String> {
    let psm = match layout {
        Layout::Block => "6",
        Layout::Auto => "3",
    };
    // Tesseract reads screen-sized text much better at twice the size; big
    // screenshots are already large enough.
    let small = std::fs::read(png).ok().and_then(|b| crate::cards::png_size(&b)).is_some_and(|(w, h)| w.max(h) <= 2000);
    let upscaled = util::runtime_dir().join(format!("kadr-ocr-2x-{}.png", std::process::id()));
    let png = if small
        && util::output::<&OsStr>(&["ffmpeg".as_ref(), "-y".as_ref(), "-loglevel".as_ref(), "error".as_ref(), "-i".as_ref(), png.as_os_str(), "-vf".as_ref(), "scale=iw*2:-1:flags=lanczos".as_ref(), upscaled.as_os_str()])
            .await
            .is_some_and(|(ok, _)| ok)
    {
        upscaled.as_path()
    } else {
        png
    };
    let langs = languages();
    let dir = tessdata_dir();
    let mut argv: Vec<&OsStr> = vec!["tesseract".as_ref()];
    if let Some(dir) = &dir {
        argv.extend(["--tessdata-dir".as_ref(), dir.as_os_str()]);
    }
    argv.push(png.as_os_str());
    let flags = ["stdout", "--oem", "1", "--psm", psm, "-l", &langs, "-c", "preserve_interword_spaces=1"];
    argv.extend(flags.iter().map(OsStr::new));
    let result = util::output(&argv).await;
    let _ = std::fs::remove_file(&upscaled);
    let (ok, text) = result?;
    let text = text.trim().to_owned();
    (ok && !text.is_empty()).then_some(text)
}

fn done(text: Option<String>, cx: &mut App) {
    if let Some(text) = &text {
        crate::platform::copy_text(text);
    }
    crate::cards::show_text(text, cx);
}

/// Fetch Tesseract's official Russian model into kadr's own folder (no root
/// needed), with the system's other models linked next to it.
pub async fn download_russian() -> Result<(), String> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or(t("ocr.no_home_folder"))?;
    let dir = base.join("kadr/tessdata");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for lang in ["eng", "osd"] {
        let system = PathBuf::from(format!("/usr/share/tessdata/{lang}.traineddata"));
        let link = dir.join(format!("{lang}.traineddata"));
        if system.exists() && !link.exists() {
            let _ = crate::platform::link(&system, &link);
        }
    }
    let out = dir.join("rus.traineddata");
    let url = "https://github.com/tesseract-ocr/tessdata_best/raw/main/rus.traineddata";
    match util::output::<&OsStr>(&["curl".as_ref(), "-fsSL".as_ref(), "-o".as_ref(), out.as_os_str(), url.as_ref()]).await {
        Some((true, _)) => Ok(()),
        _ => {
            let _ = std::fs::remove_file(&out);
            Err(t("ocr.couldnt_download_model").into())
        }
    }
}

/// A recognised word and its box, in the picture's pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    /// Block, paragraph and line: words with the same one share a line.
    pub line: (u32, u32, u32),
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

/// Every word Tesseract finds in `png`, with its box. None when Tesseract
/// couldn't run.
pub async fn words(png: &Path) -> Option<Vec<Word>> {
    // Small pictures read better at twice the size, as in `recognize`; the
    // boxes are scaled back.
    let small = std::fs::read(png).ok().and_then(|b| crate::cards::png_size(&b)).is_some_and(|(w, h)| w.max(h) <= 2000);
    let upscaled = util::runtime_dir().join(format!("kadr-words-2x-{}.png", std::process::id()));
    let scaled = small
        && util::output::<&OsStr>(&["ffmpeg".as_ref(), "-y".as_ref(), "-loglevel".as_ref(), "error".as_ref(), "-i".as_ref(), png.as_os_str(), "-vf".as_ref(), "scale=iw*2:-1:flags=lanczos".as_ref(), upscaled.as_os_str()])
            .await
            .is_some_and(|(ok, _)| ok);
    let input = if scaled { upscaled.as_path() } else { png };
    let langs = languages();
    let dir = tessdata_dir();
    let mut argv: Vec<&OsStr> = vec!["tesseract".as_ref()];
    if let Some(dir) = &dir {
        argv.extend(["--tessdata-dir".as_ref(), dir.as_os_str()]);
    }
    argv.push(input.as_os_str());
    let flags = ["stdout", "--oem", "1", "--psm", "3", "-l", &langs, "tsv"];
    argv.extend(flags.iter().map(OsStr::new));
    let result = util::output(&argv).await;
    let _ = std::fs::remove_file(&upscaled);
    let (ok, tsv) = result?;
    ok.then(|| parse_tsv(&tsv, if scaled { 0.5 } else { 1. }))
}

/// Words from Tesseract's TSV output, their boxes multiplied by `scale`.
pub(crate) fn parse_tsv(tsv: &str, scale: f32) -> Vec<Word> {
    tsv.lines()
        .skip(1)
        .filter_map(|row| {
            let cols: Vec<&str> = row.split('\t').collect();
            // level page block par line word left top width height conf text
            if cols.len() < 12 || cols[0] != "5" {
                return None;
            }
            let text = cols[11..].join("\t").trim().to_owned();
            if text.is_empty() {
                return None;
            }
            let n = |i: usize| cols[i].parse::<f32>().ok();
            let id = |i: usize| cols[i].parse::<u32>().ok();
            Some(Word { text, line: (id(2)?, id(3)?, id(4)?), left: n(6)? * scale, top: n(7)? * scale, width: n(8)? * scale, height: n(9)? * scale })
        })
        .collect()
}

#[cfg(test)]
mod word_tests {
    use super::*;

    #[test]
    fn reads_word_boxes_from_tsv() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   4\t1\t1\t1\t1\t0\t10\t20\t300\t30\t-1\t\n\
                   5\t1\t1\t1\t1\t1\t10\t20\t100\t30\t96.5\tmail\n\
                   5\t1\t1\t1\t1\t2\t120\t20\t190\t30\t91\tme@example.com\n\
                   5\t1\t1\t1\t2\t1\t10\t60\t40\t30\t-1\t \n";
        let words = parse_tsv(tsv, 0.5);
        assert_eq!(words.len(), 2);
        assert_eq!(words[1].text, "me@example.com");
        assert_eq!((words[1].left, words[1].top, words[1].width, words[1].height), (60., 10., 95., 15.));
        assert_eq!(words[0].line, words[1].line);
    }
}
