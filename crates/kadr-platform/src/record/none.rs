//! No recording backend here yet (macOS and Windows until theirs land).

use super::Spec;

pub struct Session;

impl Session {
    pub fn start(_: &Spec, _: Box<dyn FnOnce() + Send>) -> Result<Session, String> {
        Err(kadr_core::tr!("app.linux_only_for_now", kadr_core::i18n::t("common.recording")))
    }

    pub fn set_paused(&self, _: bool) {}

    pub fn stop(&self) {}
}

pub fn microphones() -> Vec<(String, String)> {
    Vec::new()
}
