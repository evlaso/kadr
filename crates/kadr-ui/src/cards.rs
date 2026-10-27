//! The stack of quick-access cards in the screen corner: which cards show,
//! for how long, and swiping one away. A card's pictures and the job behind
//! it are the app's (`view`); time comes in as an argument, so all of it is
//! tested without a window or a clock.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use kadr_core::settings::{Settings, Side};

pub struct Card<T> {
    pub id: usize,
    /// The file it shows; empty for a text card.
    pub path: PathBuf,
    /// A job still working on it: it stays until the job is done.
    pub working: bool,
    pub hovered: bool,
    /// When it goes away on its own; None stays.
    deadline: Option<Instant>,
    pub view: T,
}

pub struct CardsVm<T> {
    pub cards: Vec<Card<T>>,
    /// At most this many; a new one pushes out the oldest.
    pub max: usize,
    /// The screen edge the stack is on.
    pub side: Side,
    /// How long a card stays; None keeps it until dismissed.
    linger: Option<Duration>,
    /// Whether the pointer is over the stack.
    inside: bool,
    /// A card being swiped toward the screen edge: its id, where the press
    /// was, and how far it has moved.
    swipe: Option<(usize, f32, f32)>,
}

/// Card ids, unique for the run: work going on for a card (a job) keeps its
/// id across stacks closing and opening.
static NEXT: AtomicUsize = AtomicUsize::new(1);

impl<T> CardsVm<T> {
    pub fn new(settings: &Settings) -> CardsVm<T> {
        let linger = (settings.card_seconds > 0).then(|| Duration::from_secs(settings.card_seconds as u64));
        CardsVm { cards: Vec::new(), max: settings.max_cards.max(1), side: settings.card_side, linger, inside: false, swipe: None }
    }

    fn deadline(&self, now: Instant) -> Option<Instant> {
        self.linger.map(|d| now + d)
    }

    /// Show a card; the same file's card already showing (back from the
    /// editor, or saved over) takes the new one in its place. Its id.
    pub fn add(&mut self, path: PathBuf, working: bool, view: T, now: Instant) -> usize {
        let deadline = if working { None } else { self.deadline(now) };
        if !path.as_os_str().is_empty()
            && let Some(shown) = self.cards.iter_mut().find(|c| c.path == path)
        {
            let id = shown.id;
            *shown = Card { id, path, working, hovered: shown.hovered, deadline, view };
            return id;
        }
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        self.cards.push(Card { id, path, working, hovered: false, deadline, view });
        if self.cards.len() > self.max {
            self.cards.remove(0);
        }
        id
    }

    pub fn get_mut(&mut self, id: usize) -> Option<&mut Card<T>> {
        self.cards.iter_mut().find(|c| c.id == id)
    }

    pub fn remove(&mut self, id: usize) {
        self.cards.retain(|c| c.id != id);
    }

    /// Card `id` has work going on again (an update downloading): it stays.
    pub fn work(&mut self, id: usize) {
        if let Some(card) = self.get_mut(id) {
            card.working = true;
            card.deadline = None;
        }
    }

    /// The job behind card `id` is done: it shows `path` from now on, and
    /// goes away in time like any other.
    pub fn complete(&mut self, id: usize, path: PathBuf, now: Instant) -> bool {
        let deadline = self.deadline(now);
        let Some(card) = self.get_mut(id) else { return false };
        card.path = path;
        card.working = false;
        card.deadline = deadline;
        true
    }

    /// Cards whose time is up go, unless the pointer rests on them. True
    /// when any went.
    pub fn tick(&mut self, now: Instant) -> bool {
        let before = self.cards.len();
        let inside = self.inside;
        self.cards.retain(|c| (c.hovered && inside) || c.deadline.is_none_or(|d| now < d));
        self.cards.len() != before
    }

    pub fn hover(&mut self, id: usize, hovered: bool, now: Instant) {
        let deadline = self.deadline(now);
        if let Some(card) = self.get_mut(id) {
            card.hovered = hovered;
            if !hovered && !card.working {
                card.deadline = deadline;
            }
        }
    }

    /// The pointer over the stack or not. Leaving it gives the cards it
    /// rested on their time again.
    pub fn pointer(&mut self, inside: bool, now: Instant) {
        if self.inside && !inside {
            let deadline = self.deadline(now);
            self.cards.iter_mut().filter(|c| c.hovered && !c.working).for_each(|c| c.deadline = deadline);
        }
        self.inside = inside;
    }

    pub fn swipe_start(&mut self, id: usize, x: f32) {
        self.swipe = Some((id, x, 0.));
    }

    /// Follow the pointer toward the screen edge, never away from it. True
    /// while a card is being swiped.
    pub fn swipe_move(&mut self, x: f32) -> bool {
        let side = self.side;
        let Some((_, start, offset)) = &mut self.swipe else { return false };
        let dx = x - *start;
        *offset = if side == Side::Right { dx.max(0.) } else { dx.min(0.) };
        true
    }

    /// Let go: swiped further than `away`, the card goes (its id).
    pub fn swipe_end(&mut self, away: f32) -> Option<usize> {
        let (id, _, offset) = self.swipe.take()?;
        (offset.abs() > away).then(|| {
            self.remove(id);
            id
        })
    }

    /// How far card `id` is swiped.
    pub fn offset(&self, id: usize) -> f32 {
        self.swipe.filter(|s| s.0 == id).map_or(0., |s| s.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm(seconds: u32, max: usize) -> CardsVm<()> {
        CardsVm::new(&Settings { card_seconds: seconds, max_cards: max, ..Settings::default() })
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn cards_go_in_time_unless_rested_on() {
        let t = Instant::now();
        let mut vm = vm(10, 4);
        let a = vm.add("a.png".into(), false, (), t);
        let b = vm.add("b.png".into(), false, (), t + secs(5));
        assert!(!vm.tick(t + secs(9)));
        // The pointer rests on b: it stays past its time.
        vm.pointer(true, t + secs(9));
        vm.hover(b, true, t + secs(9));
        assert!(vm.tick(t + secs(16)));
        assert_eq!(vm.cards.iter().map(|c| c.id).collect::<Vec<_>>(), [b], "a's time was up");
        let _ = a;
        // Leaving gives it its time again.
        vm.pointer(false, t + secs(20));
        assert!(!vm.tick(t + secs(29)));
        assert!(vm.tick(t + secs(31)));
        assert!(vm.cards.is_empty());
    }

    #[test]
    fn the_same_file_takes_its_card_and_the_oldest_makes_room() {
        let t = Instant::now();
        let mut vm = vm(10, 2);
        let a = vm.add("a.png".into(), false, (), t);
        vm.add("b.png".into(), false, (), t);
        assert_eq!(vm.add("a.png".into(), false, (), t + secs(3)), a, "a again keeps its card");
        assert_eq!(vm.cards.len(), 2);
        vm.add("c.png".into(), false, (), t);
        assert_eq!(vm.cards.iter().map(|c| c.path.to_str().unwrap()).collect::<Vec<_>>(), ["b.png", "c.png"]);
    }

    #[test]
    fn a_job_card_stays_until_its_job_is_done() {
        let t = Instant::now();
        let mut vm = vm(5, 4);
        let id = vm.add("rec.mp4".into(), true, (), t);
        assert!(!vm.tick(t + secs(600)));
        assert!(vm.complete(id, "rec-studio.mp4".into(), t + secs(600)));
        assert_eq!(vm.cards[0].path, PathBuf::from("rec-studio.mp4"));
        assert!(vm.tick(t + secs(606)));
    }

    #[test]
    fn swiped_far_enough_toward_the_edge_a_card_goes() {
        let t = Instant::now();
        let mut vm = vm(0, 4);
        let id = vm.add("a.png".into(), false, (), t);
        // On the left edge: only leftwards counts.
        vm.swipe_start(id, 100.);
        vm.swipe_move(160.);
        assert_eq!(vm.offset(id), 0.);
        vm.swipe_move(70.);
        assert_eq!(vm.offset(id), -30.);
        assert_eq!(vm.swipe_end(50.), None, "not far enough");
        vm.swipe_start(id, 100.);
        vm.swipe_move(20.);
        assert_eq!(vm.swipe_end(50.), Some(id));
        assert!(vm.cards.is_empty());
        // Seconds 0: cards stay until dismissed.
        vm.add("b.png".into(), false, (), t);
        assert!(!vm.tick(t + secs(100_000)));
    }
}
