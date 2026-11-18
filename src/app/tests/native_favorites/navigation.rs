use super::*;

impl Fixture<'_> {
    #[expect(clippy::future_not_send, reason = "native foreground script")]
    pub(super) async fn deep_focus(&mut self, name: &str, state: &mut State, cx: &mut AsyncApp) {
        self.section(cx, Section::Favorites);
        self.tab_to(cx, format!("favorite-more:{}", state.favorites[0].url));
        self.press(cx, "end");
        self.settle().await;
        let url = state.favorites[19].url.clone();
        assert!(self.focused(cx, format!("favorite-more:{url}")));
        self.visible(cx, &format!("favorite-row:{url}"), "favorites-list");
        state.favorites.insert(
            0,
            entry(
                "https://www.youtube.com/watch?v=zzzzzzzzzzz",
                Some("Inserted"),
            ),
        );
        self.publish(state).await;
        assert!(self.focused(cx, format!("favorite-more:{url}")));
        self.visible(cx, &format!("favorite-row:{url}"), "favorites-list");
        self.checkpoint(&format!("{name}-inserted"), cx).await;
        state.favorites.remove(0);
        self.publish(state).await;
        assert!(self.focused(cx, format!("favorite-more:{url}")));
        self.visible(cx, &format!("favorite-row:{url}"), "favorites-list");
        self.no_commands();
        self.record(name, "At the scrolled end, insertion/removal before the focused canonical URL retains its More focus and reveals the full row; no commands.");
    }

    #[expect(clippy::future_not_send, reason = "native foreground script")]
    pub(super) async fn resize(&mut self, name: &str, cx: &mut AsyncApp) {
        self.section(cx, Section::Favorites);
        for (width, height) in [(420., 480.), (520., 560.)] {
            self.ui(cx, |window, _| window.resize(size(px(width), px(height))));
            self.settle().await;
            if width <= 480. {
                self.tab_to(cx, "collection-selector");
            } else {
                self.click(cx, "url");
            }
            let target = if width <= 480. {
                size(px(520.), px(560.))
            } else {
                size(px(420.), px(480.))
            };
            self.ui(cx, |window, _| window.resize(target));
            self.settle().await;
            self.checkpoint(&format!("{name}-from-{width:.0}"), cx)
                .await;
            self.ui(cx, |window, cx| {
                let app = self.view.read(cx);
                assert_eq!(app.section, Section::Favorites);
                assert!(
                    app.input.read(cx).focus_handle(cx).is_focused(window)
                        || (window.try_find("collection-selector").is_some()
                            && app
                                .collection_select
                                .read(cx)
                                .focus_handle(cx)
                                .is_focused(window)),
                    "resize must recover focus when collection navigation disappears"
                );
            });
        }
        self.ui(cx, |window, _| {
            window.resize(if self.width > 480. {
                size(px(520.), px(560.))
            } else {
                size(px(420.), px(480.))
            });
        });
        self.settle().await;
        self.no_commands();
        self.record(name, "Both breakpoint directions preserve Favorites; disappearing narrow chooser focus recovers, composer focus survives the return to narrow; 100/50/500 counts; no command emitted.");
    }

    #[expect(clippy::future_not_send, reason = "native foreground script")]
    pub(super) async fn breakpoint(&mut self, name: &str, cx: &mut AsyncApp) {
        self.section(cx, Section::History);
        self.click(cx, "history-clear");
        for width in [480., 481., 500., 501., 520.] {
            self.ui(cx, |window, _| window.resize(size(px(width), px(480.))));
            self.settle().await;
            self.checkpoint(&format!("{name}-{width:.0}"), cx).await;
            self.bounds(&format!("{name}-{width:.0}"), cx, true);
            self.ui(cx, |window, _| {
                assert_eq!(
                    window.try_find("collection-selector").is_some(),
                    px(width) <= crate::app::collections::COLLECTION_BREAKPOINT
                );
                assert!(window.try_find("history-clear-cancel").is_some());
            });
        }
        self.click(cx, "history-clear-cancel");
        self.ui(cx, |window, _| {
            window.resize(if self.width > 480. {
                size(px(520.), px(560.))
            } else {
                size(px(420.), px(480.))
            });
        });
        self.settle().await;
        self.no_commands();
        self.record(name, "Native boundary sweep at 480/481/500/501/520px with 100/50/500 counts and History confirmation; both actions, tabs/chooser, two full rows and footer fit.");
    }
}
