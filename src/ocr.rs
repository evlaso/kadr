//! Text recognition, like CleanShot's "Capture Text": pick an area (or use
//! a screenshot you already have), the text goes to the clipboard and a
//! card like a screenshot's shows what was found. Tesseract does the reading, with
//! every installed language that matters here; QR codes are read too (with
//! zbar, when it's installed), and links found either way can be opened.

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
        let codes = qr_codes(&png).await;
        let text = recognize(&png, Layout::Block).await;
        let _ = std::fs::remove_file(&png);
        let _ = cx.update(|cx| done(text, codes, cx));
    })
    .detach();
}

/// Read a whole picture (a screenshot card, the editor).
pub fn image(path: PathBuf, cx: &mut App) {
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let _hold = hold;
        let codes = qr_codes(&path).await;
        let text = recognize(&path, Layout::Auto).await;
        let _ = cx.update(|cx| done(text, codes, cx));
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

/// What QR codes (and bar codes) in the picture say; nothing without zbar.
async fn qr_codes(png: &Path) -> Vec<String> {
    let Some((_, out)) = util::output::<&OsStr>(&["zbarimg".as_ref(), "--quiet".as_ref(), "--raw".as_ref(), png.as_os_str()]).await else {
        return Vec::new();
    };
    out.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

/// Web links in a text: http(s):// and www. ones, without the punctuation
/// a sentence puts after them.
pub fn links(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '«' | '»')) {
        let word = word.trim_start_matches(['(', '[', '\'']).trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '\'']);
        let lower = word.to_ascii_lowercase();
        let link = if lower.starts_with("https://") || lower.starts_with("http://") {
            word.to_owned()
        } else if lower.starts_with("www.") && word.len() > 6 {
            format!("https://{word}")
        } else {
            continue;
        };
        if link.len() > "https://".len() + 3 && !found.contains(&link) {
            found.push(link);
        }
    }
    found
}

/// QR codes first (they're what a picture with one is about), then the text.
fn done(text: Option<String>, codes: Vec<String>, cx: &mut App) {
    let text = match (codes.is_empty(), text) {
        (true, text) => text,
        (false, None) => Some(codes.join("\n")),
        (false, Some(text)) => Some(format!("{}\n\n{text}", codes.join("\n"))),
    };
    if let Some(text) = &text {
        crate::platform::copy_text(text);
    }
    let link = text.as_deref().map(links).and_then(|l| l.into_iter().next());
    crate::cards::show_text(text, link, cx);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_links_in_text() {
        assert_eq!(links("see https://kadr.app/docs. Or (www.example.com), not www."), vec!["https://kadr.app/docs", "https://www.example.com"]);
        assert!(links("no links here, just http:// alone").is_empty());
    }
}
