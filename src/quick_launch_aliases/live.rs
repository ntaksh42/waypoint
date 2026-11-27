//! 開いているウィンドウとタブに付ける、メモリ上だけの別名。

use std::sync::Arc;

use windows::Win32::Foundation::HWND;

use crate::browser_tabs::Browser;
use crate::quick_launch::{Action, Entry};
use crate::window_instance::{WindowInstance, WindowMarker};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiveKey {
    Window(WindowInstance),
    Tab(WindowInstance, Browser, i32),
}

impl LiveKey {
    pub(crate) fn is_current(self) -> bool {
        match self {
            Self::Window(instance) | Self::Tab(instance, ..) => instance.is_current(),
        }
    }

    pub(crate) fn matches(self, entry: &Entry) -> bool {
        match (self, &entry.action) {
            (Self::Window(instance), Action::FocusWindow(hwnd)) => instance.hwnd == *hwnd,
            (Self::Tab(instance, browser, id), Action::FocusBrowserTab(tab)) => {
                Some(instance) == tab.source && browser == tab.browser && id == tab.tab_id
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum LiveAliasTarget {
    Window(Arc<WindowMarker>),
    Tab(WindowInstance, Browser, i32),
}

impl LiveAliasTarget {
    pub(crate) fn key(&self) -> LiveKey {
        match self {
            Self::Window(marker) => LiveKey::Window(marker.0),
            Self::Tab(source, browser, id) => LiveKey::Tab(*source, *browser, *id),
        }
    }
}

pub(super) fn supports(entry: &Entry) -> bool {
    match &entry.action {
        Action::FocusWindow(_) => true,
        Action::FocusBrowserTab(tab) => tab.source.is_some(),
        _ => false,
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LiveAliases(Vec<(LiveAliasTarget, Vec<String>)>);

impl LiveAliases {
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn begin(&self, entry: &Entry) -> Result<Option<LiveAliasTarget>, String> {
        let target =
            match &entry.action {
                Action::FocusWindow(hwnd) => {
                    if let Some((target, _)) = self.0.iter().find(|(target, _)| {
                        target.key().matches(entry) && target.key().is_current()
                    }) {
                        return Ok(Some(target.clone()));
                    }
                    let marker = WindowMarker::attach(HWND(*hwnd as *mut _))
                        .map_err(|_| "Could not attach an alias to this window".to_owned())?;
                    LiveAliasTarget::Window(Arc::new(marker))
                }
                Action::FocusBrowserTab(tab) => {
                    let source = tab
                        .source
                        .filter(|source| source.is_current())
                        .ok_or_else(|| "The tab connection is no longer active".to_owned())?;
                    LiveAliasTarget::Tab(source, tab.browser, tab.tab_id)
                }
                _ => return Ok(None),
            };
        Ok(Some(target))
    }

    pub(crate) fn names(&self, key: LiveKey) -> &[String] {
        self.0
            .iter()
            .find(|(target, _)| target.key() == key)
            .map_or(&[], |(_, aliases)| aliases)
    }

    pub(crate) fn set(&mut self, target: LiveAliasTarget, aliases: Vec<String>) {
        let key = target.key();
        self.0.retain(|(old, _)| old.key() != key);
        if !aliases.is_empty() {
            self.0.push((target, aliases));
        }
    }

    pub(crate) fn prune(&mut self, tabs: &[Entry], closed_windows: &mut Vec<isize>) -> bool {
        let original_len = self.0.len();
        self.0.retain(|(target, _)| {
            let key = target.key();
            let current = key.is_current();
            match key {
                LiveKey::Window(window) => {
                    if !current && !window.exists() {
                        closed_windows.push(window.hwnd);
                    }
                    current
                }
                LiveKey::Tab(..) => current && tabs.iter().any(|entry| key.matches(entry)),
            }
        });
        self.0.len() != original_len
    }

    pub(crate) fn matches_query(&self, query: &str) -> bool {
        if self.is_empty() || query.is_empty() || query.contains(char::is_whitespace) {
            return false;
        }
        let term = query.to_lowercase();
        self.0
            .iter()
            .any(|(_, aliases)| aliases.iter().any(|alias| alias.starts_with(&term)))
    }

    pub(crate) fn window_selection_is_stale(&self, entry: &Entry, query: &str) -> bool {
        if !matches!(entry.action, Action::FocusWindow(_)) || query.contains(char::is_whitespace) {
            return false;
        }
        let term = query.to_lowercase();
        !term.is_empty()
            && self.0.iter().any(|(target, aliases)| {
                let key = target.key();
                key.matches(entry)
                    && aliases.iter().any(|alias| alias.starts_with(&term))
                    && !key.is_current()
            })
    }

    pub(crate) fn lookup(&self, term: &str) -> (Vec<LiveKey>, Vec<LiveKey>) {
        let mut exact = Vec::new();
        let mut prefix = Vec::new();
        for (target, aliases) in &self.0 {
            if aliases.iter().any(|alias| alias == term) {
                exact.push(target.key());
            } else if aliases.iter().any(|alias| alias.starts_with(term)) {
                prefix.push(target.key());
            }
        }
        (exact, prefix)
    }
}
