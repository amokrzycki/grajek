use std::collections::VecDeque;

/// Pending entries the queue accepts before it reports itself full.
pub const QUEUE_LIMIT: usize = 100;

/// A queued video. Only the canonical URL and a known title are kept; stream URLs expire
/// and are resolved when the entry starts playing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueItem {
    /// Stable for the lifetime of the entry, so GUI actions keep targeting it while
    /// entries ahead of it start playing or are removed.
    pub id: u64,
    pub url: String,
    pub title: Option<String>,
}

impl QueueItem {
    /// The title, or the video ID until the track has been resolved.
    pub fn label(&self) -> &str {
        self.title.as_deref().unwrap_or_else(|| video_id(&self.url))
    }
}

/// `dQw4w9WgXcQ` from `https://www.youtube.com/watch?v=dQw4w9WgXcQ`.
pub fn video_id(url: &str) -> &str {
    url.rsplit_once("v=").map_or(url, |(_, id)| id)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Full;

/// FIFO of pending tracks. The controller owns the only instance.
#[derive(Default)]
pub struct Queue {
    items: VecDeque<QueueItem>,
    next_id: u64,
}

impl Queue {
    pub fn push(&mut self, url: String, title: Option<String>) -> Result<u64, Full> {
        if self.items.len() >= QUEUE_LIMIT {
            return Err(Full);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.push_back(QueueItem { id, url, title });
        Ok(id)
    }

    pub fn pop_front(&mut self) -> Option<QueueItem> {
        self.items.pop_front()
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

    fn position(&self, id: u64) -> Option<usize> {
        self.items.iter().position(|item| item.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::{Full, QUEUE_LIMIT, Queue, QueueItem, video_id};

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
        assert_eq!(queue.pop_front().map(|item| item.url), Some("u0".into()));
        assert_eq!(queue.pop_front().map(|item| item.url), Some("u1".into()));
        assert_eq!(urls(&queue), ["u2"]);
        queue.pop_front();
        assert!(queue.snapshot().is_empty() && queue.pop_front().is_none());
    }

    #[test]
    fn ids_are_never_reused() {
        let mut queue = Queue::default();
        let first = queue.push("a".into(), None);
        queue.pop_front();
        assert_ne!(first, queue.push("b".into(), None));
    }

    #[test]
    fn holds_at_most_the_limit() {
        let mut queue = filled(QUEUE_LIMIT);
        assert_eq!(queue.push("extra".into(), None), Err(Full));
        queue.pop_front();
        assert!(queue.push("extra".into(), None).is_ok());
        assert_eq!(queue.snapshot().len(), QUEUE_LIMIT);
    }

    #[test]
    fn removes_by_id_even_after_positions_change() {
        let mut queue = filled(4);
        let ids: Vec<u64> = queue.snapshot().iter().map(|item| item.id).collect();
        queue.pop_front();
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
    fn untitled_entries_show_the_video_id() {
        let url = "https://www.youtube.com/watch?v=BaW_jenozKc";
        assert_eq!(video_id(url), "BaW_jenozKc");
        let mut item = QueueItem {
            id: 0,
            url: url.into(),
            title: None,
        };
        assert_eq!(item.label(), "BaW_jenozKc");
        item.title = Some("Song".into());
        assert_eq!(item.label(), "Song");
    }
}
