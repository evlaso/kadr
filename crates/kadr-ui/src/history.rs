//! Undo and redo as changes, not copies of the drawing: each step keeps only
//! the shapes it took out and put in (at one place in the list) and the
//! document's own fields if they changed.
//!
//! The editor calls `checkpoint` before it changes the drawing, as before;
//! what changed is worked out at the next checkpoint, undo or redo, by
//! comparing with the drawing as it was then. So the only whole copy is the
//! one of the step in progress.

use kadr_core::document::{Doc, Orientation, P, Shape};

/// The document's fields besides its shapes.
#[derive(Clone, Debug, PartialEq)]
struct Fields {
    crop: Option<(P, P)>,
    view: Orientation,
    scale: f32,
    export_density: Option<f64>,
    fill: Option<[u8; 4]>,
}

impl Fields {
    fn of(doc: &Doc) -> Fields {
        Fields { crop: doc.crop, view: doc.view, scale: doc.scale, export_density: doc.export_density, fill: doc.fill }
    }

    fn put(&self, doc: &mut Doc) {
        (doc.crop, doc.view, doc.scale, doc.fill) = (self.crop, self.view, self.scale, self.fill);
        doc.export_density = self.export_density;
    }
}

/// One step: `removed` taken out at `at` and `inserted` put there, and the
/// fields before and after when they changed.
#[derive(Clone, Debug)]
pub struct Change {
    at: usize,
    removed: Vec<Shape>,
    inserted: Vec<Shape>,
    fields: Option<(Fields, Fields)>,
}

impl Change {
    /// What turns `before` into `after`; None when they're the same.
    fn between(before: &Doc, after: &Doc) -> Option<Change> {
        let (old, new) = (&before.shapes, &after.shapes);
        let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let room = old.len().min(new.len()) - prefix;
        let suffix = old.iter().rev().zip(new.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
        let removed = old[prefix..old.len() - suffix].to_vec();
        let inserted = new[prefix..new.len() - suffix].to_vec();
        let (was, now) = (Fields::of(before), Fields::of(after));
        let fields = (was != now).then_some((was, now));
        (!removed.is_empty() || !inserted.is_empty() || fields.is_some()).then_some(Change { at: prefix, removed, inserted, fields })
    }

    fn redo(&self, doc: &mut Doc) {
        doc.shapes.splice(self.at..self.at + self.removed.len(), self.inserted.iter().cloned());
        if let Some((_, now)) = &self.fields {
            now.put(doc);
        }
    }

    fn undo(&self, doc: &mut Doc) {
        doc.shapes.splice(self.at..self.at + self.inserted.len(), self.removed.iter().cloned());
        if let Some((was, _)) = &self.fields {
            was.put(doc);
        }
    }

    fn shapes(&self) -> impl Iterator<Item = &Shape> {
        self.removed.iter().chain(&self.inserted)
    }
}

#[derive(Default)]
pub struct History {
    undo: Vec<Change>,
    redo: Vec<Change>,
    /// The drawing at the last checkpoint, until its change is worked out.
    pending: Option<Doc>,
}

impl History {
    /// Turn the step in progress into a change.
    fn settle(&mut self, doc: &Doc) {
        if let Some(before) = self.pending.take()
            && let Some(change) = Change::between(&before, doc)
        {
            self.undo.push(change);
        }
    }

    /// Before changing `doc`: what undo will go back to.
    pub fn checkpoint(&mut self, doc: &Doc) {
        self.settle(doc);
        self.pending = Some(doc.clone());
        self.redo.clear();
    }

    /// The last checkpoint is no step after all.
    pub fn drop_last(&mut self) {
        if self.pending.take().is_none() {
            self.undo.pop();
        }
    }

    /// The depth now, to `rewind` to later: the step in progress settled.
    pub fn mark(&mut self, doc: &Doc) -> usize {
        self.settle(doc);
        self.undo.len()
    }

    pub fn undo(&mut self, doc: &mut Doc) -> bool {
        self.settle(doc);
        let Some(change) = self.undo.pop() else { return false };
        change.undo(doc);
        self.redo.push(change);
        true
    }

    pub fn redo(&mut self, doc: &mut Doc) -> bool {
        self.settle(doc);
        let Some(change) = self.redo.pop() else { return false };
        change.redo(doc);
        self.undo.push(change);
        true
    }

    /// Steps to undo (the one in progress counts).
    pub fn depth(&self) -> usize {
        self.undo.len() + self.pending.is_some() as usize
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Back to `depth` steps, the drawing having been put back as it was
    /// then by the caller (a cancelled crop).
    pub fn rewind(&mut self, depth: usize) {
        self.pending = None;
        self.undo.truncate(depth);
        self.redo.clear();
    }

    /// Every shape the history holds, for caches to keep.
    pub fn shapes(&self) -> impl Iterator<Item = &Shape> {
        self.undo.iter().chain(&self.redo).flat_map(Change::shapes).chain(self.pending.iter().flat_map(|d| d.shapes.iter()))
    }

    /// The shapes kept, counted: what the history costs, for tests.
    pub fn kept(&self) -> usize {
        self.undo.iter().chain(&self.redo).map(|c| c.removed.len() + c.inserted.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use kadr_core::color::Rgba;
    use kadr_core::document::{Kind, TextStyle};

    use super::*;

    fn shape(n: f32) -> Shape {
        Shape { kind: Kind::Rect, a: (n, n), b: (n + 10., n + 10.), color: Rgba::WHITE, size: 2., filled: false, text: TextStyle::Plain, bend: None }
    }

    /// A few kinds of edit, each after a checkpoint, like the editor does.
    fn edits() -> Vec<Box<dyn Fn(&mut Doc)>> {
        vec![
            Box::new(|d| d.shapes.push(shape(1.))),
            Box::new(|d| d.shapes.push(shape(2.))),
            Box::new(|d| d.shapes[0].translate((5., 5.))),
            Box::new(|d| d.crop = Some(((0., 0.), (50., 40.)))),
            Box::new(|d| {
                d.shapes.remove(0);
            }),
            Box::new(|d| d.shapes.insert(0, shape(9.))),
            Box::new(|d| d.view = d.view.turned()),
            Box::new(|d| d.export_density = Some(2.)),
            Box::new(|d| d.shapes.clear()),
        ]
    }

    #[test]
    fn undoing_everything_gives_back_the_start_and_redo_the_end() {
        let start = Doc { shapes: vec![shape(0.)], ..Doc::default() };
        let mut doc = start.clone();
        let mut h = History::default();
        let mut states = vec![doc.clone()];
        for edit in edits() {
            h.checkpoint(&doc);
            edit(&mut doc);
            states.push(doc.clone());
        }
        let end = doc.clone();
        for want in states.iter().rev().skip(1) {
            assert!(h.undo(&mut doc));
            assert_eq!(&doc, want);
        }
        assert!(!h.undo(&mut doc));
        assert_eq!(doc, start);
        while h.redo(&mut doc) {}
        assert_eq!(doc, end);
    }

    #[test]
    fn a_step_keeps_only_what_it_changed() {
        let mut doc = Doc { shapes: (0..50).map(|i| shape(i as f32)).collect(), ..Doc::default() };
        let mut h = History::default();
        h.checkpoint(&doc);
        doc.shapes[20].translate((1., 0.));
        h.checkpoint(&doc);
        doc.shapes.push(shape(99.));
        h.checkpoint(&doc);
        doc.crop = Some(((0., 0.), (5., 5.)));
        h.checkpoint(&doc);
        // The 50 shapes aren't copied into each step: a moved one is two
        // (before and after), an added one is one, a crop is none.
        assert_eq!(h.kept(), 3);
    }

    #[test]
    fn a_new_edit_drops_what_could_be_redone() {
        let mut doc = Doc::default();
        let mut h = History::default();
        h.checkpoint(&doc);
        doc.shapes.push(shape(1.));
        assert!(h.undo(&mut doc));
        assert_eq!(h.redo_depth(), 1);
        h.checkpoint(&doc);
        doc.shapes.push(shape(2.));
        assert_eq!(h.redo_depth(), 0);
        assert!(h.undo(&mut doc));
        assert!(doc.shapes.is_empty());
    }

    #[test]
    fn a_dropped_checkpoint_leaves_no_step() {
        let mut doc = Doc::default();
        let mut h = History::default();
        h.checkpoint(&doc);
        doc.shapes.push(shape(1.));
        h.checkpoint(&doc);
        h.drop_last();
        assert_eq!(h.depth(), 1);
        assert!(h.undo(&mut doc));
        assert!(doc.shapes.is_empty());
    }

    #[test]
    fn rewinding_goes_back_to_a_depth() {
        let mut doc = Doc::default();
        let mut h = History::default();
        h.checkpoint(&doc);
        doc.shapes.push(shape(1.));
        // A crop opens with the shape's step still in progress, and nothing
        // is changed before it's cancelled.
        let entry = (doc.clone(), h.mark(&doc));
        doc = entry.0.clone();
        h.rewind(entry.1);
        assert_eq!(h.depth(), 1, "the shape's step survives a crop that changed nothing");
        // One that changed things goes back to the shape alone.
        let entry = (doc.clone(), h.mark(&doc));
        h.checkpoint(&doc);
        doc.crop = Some(((0., 0.), (1., 1.)));
        h.checkpoint(&doc);
        doc.crop = Some(((0., 0.), (2., 2.)));
        doc = entry.0;
        h.rewind(entry.1);
        assert_eq!(h.depth(), 1);
        assert!(h.undo(&mut doc));
        assert!(doc.shapes.is_empty());
    }
}
