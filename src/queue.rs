use std::collections::VecDeque;

/// Pending entries the queue accepts before it reports itself full.
pub const QUEUE_LIMIT: usize = 100;

pub(crate) fn is_full(len: usize) -> bool {
    len >= QUEUE_LIMIT
}

/// Capacity refusal that must carry the next action in the same sentence.
pub(crate) fn full_message(action: &str) -> String {
    format!("Queue full ({QUEUE_LIMIT} tracks). {action}")
}

pub(crate) fn full_notice() -> String {
    format!("The queue is full ({QUEUE_LIMIT} tracks).")
}

/// Lowercase fragment spliced into a longer sentence.
pub(crate) fn limit_note() -> String {
    format!("queue limit {QUEUE_LIMIT}")
}

/// A queued video. Only the canonical URL and a known title are kept; stream URLs expire
/// and are resolved when the entry starts playing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueItem {
    /// Stable for the lifetime of the entry, so GUI actions keep targeting it while
    /// entries ahead of it start playing or are removed.
    pub id: u64,
    pub url: String,
    pub title: Option<String>,
    /// Transient lookup outcome; unresolved entries retry after restoration.
    pub title_unavailable: bool,
}

impl QueueItem {
    /// A truthful loading or failure label until a title is known.
    pub fn label(&self) -> &str {
        self.title.as_deref().unwrap_or(if self.title_unavailable {
            "Title unavailable"
        } else {
            "Loading title…"
        })
    }

    /// Hover text explaining why an entry still lacks its title.
    pub fn tooltip(&self) -> String {
        match &self.title {
            Some(title) => title.clone(),
            None if self.title_unavailable => format!(
                "Could not load the title. Playback can still be attempted. {}",
                self.url
            ),
            None => format!("Waiting for YouTube metadata. {}", self.url),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Full;

/// Where a new pending entry lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    End,
    Next,
}

/// FIFO of pending tracks. The controller owns the only instance.
#[derive(Default)]
pub struct Queue {
    items: VecDeque<QueueItem>,
    next_id: u64,
}

impl Queue {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn is_full(&self) -> bool {
        is_full(self.items.len())
    }

    pub fn push(&mut self, url: String, title: Option<String>) -> Result<u64, Full> {
        self.insert(url, title, Placement::End)
    }

    pub fn insert(
        &mut self,
        url: String,
        title: Option<String>,
        placement: Placement,
    ) -> Result<u64, Full> {
        if self.is_full() {
            return Err(Full);
        }
        let id = self.next_id;
        self.next_id += 1;
        let item = QueueItem {
            id,
            url,
            title,
            title_unavailable: false,
        };
        match placement {
            Placement::End => self.items.push_back(item),
            Placement::Next => self.items.push_front(item),
        }
        Ok(id)
    }

    /// Makes an existing entry the first pending one. False only for an unknown ID; moving
    /// never changes the length, so a full queue is no obstacle.
    pub fn move_to_front(&mut self, id: u64) -> bool {
        let Some(index) = self.position(id) else {
            return false;
        };
        if let Some(item) = self.items.remove(index) {
            self.items.push_front(item);
        }
        true
    }

    /// Appends as many entries as currently free slots allow; returns how many fit.
    pub fn append(&mut self, entries: impl IntoIterator<Item = (String, Option<String>)>) -> usize {
        let available = QUEUE_LIMIT.saturating_sub(self.items.len());
        let mut added = 0;
        for (url, title) in entries.into_iter().take(available) {
            if self.push(url, title).is_ok() {
                added += 1;
            }
        }
        added
    }

    pub fn title_of(&self, url: &str) -> Option<String> {
        self.items
            .iter()
            .find(|item| item.url == url && item.title.is_some())
            .and_then(|item| item.title.clone())
    }

    pub fn remove(&mut self, id: u64) -> bool {
        self.position(id)
            .is_some_and(|index| self.items.remove(index).is_some())
    }

    /// Moves one entry a single place earlier (`up`) or later. Edges and unknown IDs are no-ops.
    pub fn shift(&mut self, id: u64, up: bool) -> bool {
        let Some(index) = self.position(id) else {
            return false;
        };
        let target = if up {
            index.checked_sub(1)
        } else {
            Some(index + 1).filter(|target| *target < self.items.len())
        };
        target.is_some_and(|target| {
            self.items.swap(index, target);
            true
        })
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn snapshot(&self) -> Vec<QueueItem> {
        self.items.iter().cloned().collect()
    }

    /// Apply only to the exact entries that requested this lookup, wherever they moved.
    pub fn resolve_titles(&mut self, ids: &[u64], title: Option<&str>) {
        for item in &mut self.items {
            if ids.contains(&item.id) && item.title.is_none() {
                item.title = title.map(str::to_owned);
                item.title_unavailable = title.is_none();
            }
        }
    }

    fn position(&self, id: u64) -> Option<usize> {
        self.items.iter().position(|item| item.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::{Full, Placement, QUEUE_LIMIT, Queue, QueueItem};

    fn pop_front(queue: &mut Queue) -> Option<QueueItem> {
        let item = queue.snapshot().into_iter().next()?;
        queue.remove(item.id).then_some(item)
    }

    fn urls(queue: &Queue) -> Vec<String> {
        queue.snapshot().into_iter().map(|item| item.url).collect()
    }

    fn filled(count: usize) -> Queue {
        let mut queue = Queue::default();
        for index in 0..count {
            assert!(queue.push(format!("u{index}"), None).is_ok());
        }
        queue
    }

    #[test]
    fn is_first_in_first_out() {
        let mut queue = filled(3);
        assert_eq!(
            pop_front(&mut queue).map(|item| item.url),
            Some("u0".into())
        );
        assert_eq!(
            pop_front(&mut queue).map(|item| item.url),
            Some("u1".into())
        );
        assert_eq!(urls(&queue), ["u2"]);
        pop_front(&mut queue);
        assert!(queue.snapshot().is_empty() && pop_front(&mut queue).is_none());
    }

    #[test]
    fn ids_are_never_reused() {
        let mut queue = Queue::default();
        let first = queue.push("a".into(), None);
        pop_front(&mut queue);
        assert_ne!(first, queue.push("b".into(), None));
    }

    #[test]
    fn holds_at_most_the_limit() {
        let mut queue = filled(QUEUE_LIMIT);
        assert_eq!(queue.push("extra".into(), None), Err(Full));
        pop_front(&mut queue);
        assert!(queue.push("extra".into(), None).is_ok());
        assert_eq!(queue.snapshot().len(), QUEUE_LIMIT);
    }

    #[test]
    fn removes_by_id_even_after_positions_change() {
        let mut queue = filled(4);
        let ids: Vec<u64> = queue.snapshot().iter().map(|item| item.id).collect();
        pop_front(&mut queue);
        assert!(!queue.remove(ids[0]), "already started entries are gone");
        assert!(queue.remove(ids[2]));
        assert!(!queue.remove(ids[2]));
        assert_eq!(urls(&queue), ["u1", "u3"]);
    }

    #[test]
    fn moves_one_place_and_stops_at_the_edges() {
        let mut queue = filled(3);
        let ids: Vec<u64> = queue.snapshot().iter().map(|item| item.id).collect();
        assert!(!queue.shift(ids[0], true));
        assert!(!queue.shift(ids[2], false));
        assert!(queue.shift(ids[2], true));
        assert_eq!(urls(&queue), ["u0", "u2", "u1"]);
        assert!(queue.shift(ids[0], false));
        assert_eq!(urls(&queue), ["u2", "u0", "u1"]);
        assert!(!queue.shift(99, true));
    }

    #[test]
    fn clear_empties_the_queue() {
        let mut queue = filled(5);
        queue.clear();
        assert_eq!(queue.snapshot().len(), 0);
        assert!(queue.push("again".into(), None).is_ok());
    }

    #[test]
    fn late_out_of_order_titles_follow_ids_and_never_resurrect_entries() {
        let mut queue = filled(3);
        let items = queue.snapshot();
        queue.shift(items[2].id, true);
        queue.resolve_titles(&[items[2].id], Some("Third"));
        queue.resolve_titles(&[items[0].id], Some("First"));
        assert_eq!(queue.snapshot()[1].label(), "Third");
        assert_eq!(queue.snapshot()[0].label(), "First");
        queue.remove(items[1].id);
        queue.resolve_titles(&[items[1].id], Some("Removed"));
        assert_eq!(queue.snapshot().len(), 2);
        queue.clear();
        let new = queue.push(items[0].url.clone(), None);
        queue.resolve_titles(&[items[0].id], Some("Stale"));
        assert_ne!(new, Ok(items[0].id));
        assert_eq!(queue.snapshot()[0].title, None);
    }

    fn ids(queue: &Queue) -> Vec<u64> {
        queue.snapshot().iter().map(|item| item.id).collect()
    }

    #[test]
    fn next_inserts_at_the_front_with_a_new_id() {
        let mut queue = filled(2);
        let before = ids(&queue);
        let id = queue.insert("x".into(), Some("X".into()), Placement::Next);
        assert!(matches!(id, Ok(id) if !before.contains(&id)));
        assert_eq!(urls(&queue), ["x", "u0", "u1"]);
        assert_eq!(queue.snapshot()[0].title.as_deref(), Some("X"));
        assert_eq!(
            queue.insert("y".into(), None, Placement::End).map(|_| ()),
            Ok(())
        );
        assert_eq!(urls(&queue), ["x", "u0", "u1", "y"]);
    }

    #[test]
    fn next_keeps_duplicates_as_distinct_entries() {
        let mut queue = filled(1);
        let id = queue.insert("u0".into(), None, Placement::Next);
        assert_eq!(urls(&queue), ["u0", "u0"]);
        let ids = ids(&queue);
        assert_ne!(ids[0], ids[1]);
        assert_eq!(id, Ok(ids[0]));
    }

    #[test]
    fn next_is_refused_at_the_limit_without_mutation() {
        let mut queue = filled(QUEUE_LIMIT);
        let before = queue.snapshot();
        assert_eq!(queue.insert("x".into(), None, Placement::Next), Err(Full));
        assert_eq!(queue.snapshot(), before);
        pop_front(&mut queue);
        assert!(queue.insert("x".into(), None, Placement::Next).is_ok());
        assert_eq!(queue.snapshot().len(), QUEUE_LIMIT);
        assert_eq!(queue.snapshot()[0].url, "x");
    }

    #[test]
    fn move_to_front_keeps_the_entry_and_the_order_of_the_rest() {
        let mut queue = filled(4);
        let items = queue.snapshot();
        queue.resolve_titles(&[items[2].id], Some("Third"));
        queue.resolve_titles(&[items[3].id], None);
        let moved = queue.snapshot()[3].clone();
        assert!(queue.move_to_front(moved.id));
        assert_eq!(urls(&queue), ["u3", "u0", "u1", "u2"]);
        assert_eq!(queue.snapshot()[0], moved);
        assert!(moved.title_unavailable);
        assert!(queue.move_to_front(items[2].id));
        assert_eq!(urls(&queue), ["u2", "u3", "u0", "u1"]);
        assert_eq!(queue.snapshot()[0].title.as_deref(), Some("Third"));
    }

    #[test]
    fn move_to_front_of_the_first_or_an_unknown_entry_changes_nothing() {
        let mut queue = filled(3);
        let first = queue.snapshot()[0].id;
        assert!(queue.move_to_front(first));
        assert_eq!(urls(&queue), ["u0", "u1", "u2"]);
        assert!(!queue.move_to_front(999));
        pop_front(&mut queue);
        assert!(!queue.move_to_front(first), "a consumed ID is gone");
        assert_eq!(urls(&queue), ["u1", "u2"]);
    }

    #[test]
    fn move_to_front_works_when_the_queue_is_full() {
        let mut queue = filled(QUEUE_LIMIT);
        let last = queue.snapshot()[QUEUE_LIMIT - 1].id;
        assert!(queue.move_to_front(last));
        assert_eq!(queue.snapshot().len(), QUEUE_LIMIT);
        assert_eq!(queue.snapshot()[0].id, last);
    }

    #[test]
    fn a_late_title_follows_the_moved_entry() {
        let mut queue = filled(3);
        let last = queue.snapshot()[2].id;
        queue.move_to_front(last);
        queue.resolve_titles(&[last], Some("Moved"));
        assert_eq!(queue.snapshot()[0].label(), "Moved");
        assert_eq!(queue.snapshot()[1].title, None);
    }

    #[test]
    fn untitled_entries_show_truthful_lookup_states() {
        let mut item = QueueItem {
            id: 0,
            url: "https://www.youtube.com/watch?v=BaW_jenozKc".into(),
            title: None,
            title_unavailable: false,
        };
        assert_eq!(item.label(), "Loading title…");
        assert_eq!(
            item.tooltip(),
            "Waiting for YouTube metadata. https://www.youtube.com/watch?v=BaW_jenozKc"
        );
        item.title_unavailable = true;
        assert_eq!(item.label(), "Title unavailable");
        assert_eq!(
            item.tooltip(),
            "Could not load the title. Playback can still be attempted. https://www.youtube.com/watch?v=BaW_jenozKc"
        );
        item.title = Some("Song".into());
        assert_eq!(item.label(), "Song");
        assert_eq!(item.tooltip(), "Song");
    }
}
