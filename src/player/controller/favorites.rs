use super::{Backend, Controller};
use crate::{
    config::video_url,
    player::{FavoriteRefusal, FavoriteResult, SearchPhase},
    search::ResultItem,
    store::{FAVORITES_LIMIT, FavoriteEntry},
};

impl<B: Backend> Controller<B> {
    /// Applies a non-search Save/Remove. The URL is canonicalized first; a bad URL is refused
    /// without touching membership, playback or the queue.
    pub(super) fn set_favorite(&mut self, id: u64, url: &str, saved: bool) {
        if self.stale_favorite_request(id) {
            return self.sync();
        }
        let Ok(canonical) = video_url(url) else {
            self.acknowledge_favorite(id, url.to_owned(), saved, Err(FavoriteRefusal::InvalidUrl));
            return;
        };
        let entry = self.known_facts(&canonical);
        self.apply_favorite(id, canonical, saved, entry);
    }

    /// Applies a search Save/Remove. The session and selected result are validated first; the
    /// selected result's facts are copied, then still-missing fields fall back to known facts
    /// for the same canonical URL. Called from `search_action`.
    pub(super) fn save_search_favorite(
        &mut self,
        id: u64,
        search_id: u64,
        video_id: &str,
        saved: bool,
    ) {
        if self.stale_favorite_request(id) {
            return self.sync();
        }
        let Some(item) = self.selected_result(search_id, video_id) else {
            self.acknowledge_favorite(
                id,
                video_id.to_owned(),
                saved,
                Err(FavoriteRefusal::StaleSearch),
            );
            return;
        };
        let Ok(canonical) = video_url(&item.url) else {
            self.acknowledge_favorite(
                id,
                video_id.to_owned(),
                saved,
                Err(FavoriteRefusal::StaleSearch),
            );
            return;
        };
        let facts = self.known_facts(&canonical);
        let entry = FavoriteEntry {
            url: canonical.clone(),
            title: item.title.clone().or(facts.title),
            channel: item.channel.clone().or(facts.channel),
            duration_secs: item
                .duration
                .map(|duration| duration.as_secs())
                .or(facts.duration_secs),
            live: item.live,
        };
        self.apply_favorite(id, canonical, saved, entry);
    }

    /// Whether an older or duplicate request id must be ignored.
    fn stale_favorite_request(&self, id: u64) -> bool {
        self.favorite_result
            .as_ref()
            .is_some_and(|result| result.request_id >= id)
    }

    fn selected_result(&self, search_id: u64, video_id: &str) -> Option<ResultItem> {
        if search_id != self.search.state.id {
            return None;
        }
        let SearchPhase::Results(results) = &self.search.state.phase else {
            return None;
        };
        results
            .items
            .iter()
            .find(|item| item.id() == video_id)
            .cloned()
    }

    /// Known facts for a canonical URL: current playback, then history, then queue. Channel
    /// and live have no non-search source and stay absent.
    fn known_facts(&self, url: &str) -> FavoriteEntry {
        let playback = self.track.as_ref().filter(|track| track.request.url == url);
        let history = self.saved.history.iter().find(|entry| entry.url == url);
        let title = playback
            .and_then(|track| track.info.title.clone())
            .or_else(|| history.and_then(|entry| entry.title.clone()))
            .or_else(|| self.queue.title_of(url));
        let duration_secs = playback
            .and_then(|track| track.info.duration.map(|duration| duration.as_secs()))
            .or_else(|| history.and_then(|entry| entry.duration_secs));
        FavoriteEntry {
            url: url.to_owned(),
            title,
            channel: None,
            duration_secs,
            live: false,
        }
    }

    fn apply_favorite(&mut self, id: u64, canonical: String, saved: bool, entry: FavoriteEntry) {
        // Runtime titles and channels obey the same bounds as persisted data.
        let entry = FavoriteEntry {
            title: entry.title.as_deref().and_then(crate::store::safe_title),
            channel: entry.channel.as_deref().and_then(crate::store::safe_title),
            ..entry
        };
        if let Err(refusal) = self.mutate_favorites(entry, saved) {
            self.acknowledge_favorite(id, canonical, saved, Err(refusal));
        } else {
            // An accepted mutation always persists: re-issuing a desired state (including
            // a no-op Remove of an already-absent entry) reschedules the saver to retry an
            // earlier failed write, without changing membership or order.
            self.persist();
            self.acknowledge_favorite(id, canonical, saved, Ok(()));
        }
    }

    fn mutate_favorites(
        &mut self,
        entry: FavoriteEntry,
        saved: bool,
    ) -> Result<(), FavoriteRefusal> {
        let existing = self
            .saved
            .favorites
            .iter()
            .position(|known| known.url == entry.url);
        let Some(index) = existing else {
            if !saved {
                return Ok(());
            }
            if self.saved.favorites.len() >= FAVORITES_LIMIT {
                return Err(FavoriteRefusal::Full);
            }
            self.saved.favorites.insert(0, entry);
            return Ok(());
        };
        if saved {
            let known = &mut self.saved.favorites[index];
            if known.title.is_none() && entry.title.is_some() {
                known.title = entry.title;
            }
            if known.channel.is_none() && entry.channel.is_some() {
                known.channel = entry.channel;
            }
            if known.duration_secs.is_none() && entry.duration_secs.is_some() {
                known.duration_secs = entry.duration_secs;
            }
            if !known.live && entry.live {
                known.live = true;
            }
            Ok(())
        } else {
            self.saved.favorites.remove(index);
            Ok(())
        }
    }

    fn acknowledge_favorite(
        &mut self,
        request_id: u64,
        url: String,
        saved: bool,
        result: Result<(), FavoriteRefusal>,
    ) {
        self.favorite_result = Some(FavoriteResult {
            request_id,
            url,
            saved,
            result,
        });
        self.sync();
    }
}
