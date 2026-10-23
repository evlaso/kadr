//! kadr's global hotkeys, set in the settings. On Linux they are Hyprland
//! binds that kadr writes to `~/.config/hypr/kadr-keys.lua`, which the hook
//! `data/hypr/kadr.lua` loads. The first save takes kadr's binds over from
//! `bindings.lua`: they are commented out there (after a backup), so nothing
//! is bound twice. On macOS and Windows the same lines go to kadr's own
//! config folder, and `kadr tray` registers them with the system.

pub use kadr_core::hotkeys::*;

/// The Hyprland submap kadr switches to while a shortcut is being recorded:
/// it has no real binds, so the keys pressed reach the settings instead of
/// firing whatever they're bound to.
const QUIET: &str = "kadr-keys";

/// Write kadr's binds and have Hyprland (or `kadr tray`) pick them up.
pub fn save(keys: &[Vec<String>]) -> std::io::Result<()> {
    let first = !kadr_core::hotkeys::own_file().exists();
    if let Some(dir) = kadr_core::hotkeys::own_file().parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut out = String::from("-- kadr's hotkeys, written by its settings (Shortcuts). Loaded by hypr/kadr.lua.\n");
    for (action, specs) in ACTIONS.iter().zip(keys) {
        for spec in specs {
            out.push_str(&format!("o.bind(\"{spec}\", \"{} (kadr)\", \"{}\")\n", crate::i18n::t(action.title), action.command));
        }
    }
    std::fs::write(kadr_core::hotkeys::own_file(), out)?;
    if !cfg!(target_os = "linux") {
        kadr_tray::restart();
        return Ok(());
    }
    if first {
        retire_old_binds()?;
    }
    reload();
    Ok(())
}

/// Comment out kadr's binds in bindings.lua, keeping a backup.
fn retire_old_binds() -> std::io::Result<()> {
    let path = kadr_core::hotkeys::hypr_dir().join("bindings.lua");
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(()) };
    if !text.lines().any(|l| parse_bind(l).is_some_and(|(_, c)| c.starts_with("kadr "))) {
        return Ok(());
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    std::fs::write(kadr_core::hotkeys::hypr_dir().join(format!("bindings.lua.bak-{stamp}")), &text)?;
    let out: Vec<String> = text
        .lines()
        .map(|l| match parse_bind(l) {
            Some((_, c)) if c.starts_with("kadr ") => format!("-- moved to kadr-keys.lua: {l}"),
            _ => l.to_owned(),
        })
        .collect();
    std::fs::write(&path, out.join("\n") + "\n")
}

fn hyprctl(lua: &str) {
    let _ = std::process::Command::new("hyprctl").args(["eval", lua]).output();
}

fn reload() {
    let _ = std::process::Command::new("hyprctl").arg("reload").output();
}

/// Pause every bind while a shortcut is recorded (`false` brings them back).
pub fn quiet(on: bool) {
    // Elsewhere the system holds the keys while `kadr tray` runs.
    if !cfg!(target_os = "linux") {
        return if on { kadr_tray::stop() } else { kadr_tray::restart() };
    }
    if on {
        hyprctl(&format!(
            r#"hl.define_submap("{QUIET}", function() hl.bind("SUPER + CTRL + ALT + SHIFT + F24", hl.dsp.submap("reset")) end); hl.dispatch(hl.dsp.submap("{QUIET}"))"#
        ));
    } else {
        hyprctl(r#"hl.dispatch(hl.dsp.submap("reset"))"#);
    }
}
