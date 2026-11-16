use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;
use space_core::{scan_path_with_options, NodeId, ScanProgress, SizeMode, SpaceTree};

use crate::anim::{TransKind, Transition};
use crate::review;
use crate::views::ViewMode;
use crate::volumes;

use super::workers::spawn_bg;
use super::{
    find_path, is_live_node, BasketItem, BgMsg, DeleteSource, Mode, RescanSavedPaths, SpaceApp,
};

impl SpaceApp {
    pub(super) fn invalidate_largest(&mut self) {
        self.largest.invalidate();
        self.geom = None;
    }

    pub(super) fn start_scan(&mut self, path: PathBuf, force: bool) {
        self.mode = Mode::Explore;
        self.choosing_target = false;
        if let Some(scan) = self.scan.take() {
            scan.cancel();
        }

        if !force {
            if let Some(cache) = self.cache.clone() {
                self.busy = true;
                self.status = crate::tr!("Loading cache for {}…", path.display());
                self.tree = None;
                self.progress = None;
                self.caches.invalidate_all();
                self.invalidate_largest();
                let tx = self.bg_tx.clone();
                let path_bg = path.clone();
                let ttl_days = self.settings.cache_ttl_days;
                let _ = spawn_bg("space-cache-load", move || {
                    let loaded = cache
                        .load_with_ttl(&path_bg, ttl_days)
                        .ok()
                        .map(|l| (Arc::new(l.tree), l.stale, l.offline));
                    let _ = tx.send(BgMsg::CacheResult {
                        path: path_bg,
                        loaded,
                    });
                });
                return;
            }
        }

        self.begin_filesystem_scan(path);
    }

    pub(super) fn begin_filesystem_scan(&mut self, path: PathBuf) {
        self.from_cache = false;
        self.cache_stale = false;
        self.cache_offline = false;
        self.skipped_expanded = false;
        self.busy = false;
        self.tree = None;
        self.review.clear();
        self.history_back.clear();
        self.history_fwd.clear();
        self.reset_tree_view_state();
        self.caches.invalidate_all();
        self.invalidate_largest();
        self.progress = Some(ScanProgress {
            files_seen: 0,
            bytes_seen: 0,
            current_path: path.display().to_string(),
            skipped_errors: 0,
        });
        self.status = crate::tr!("Scanning {}…", path.display());
        self.scan = Some(scan_path_with_options(path, self.settings.scan_options()));
    }

    pub(super) fn rescan_current(&mut self) {
        let Some(tree) = &self.tree else { return };
        let path = PathBuf::from(&tree.root_path);
        self.start_scan(path, true);
    }

    pub(super) fn navigate(&mut self, id: NodeId, push_history: bool) {
        let Some(tree) = &self.tree else { return };
        if let Some(target_node) = tree.try_node(id) {
            if !target_node.is_dir {
                return;
            }
            if id == self.focus {
                return;
            }
            let zoom_in = tree.path_to(id).contains(&self.focus) && id != self.focus;
            // Child of current focus → zoom in; otherwise zoom out / sideways.
            let kind = if zoom_in || target_node.parent.is_some_and(|p| p == self.focus) {
                TransKind::ZoomIn
            } else {
                TransKind::ZoomOut
            };
            if push_history {
                self.history_back.push(self.focus);
                self.history_fwd.clear();
            }
            self.focus = id;
            self.clear_selection();
            self.hovered = None;
            self.transition = if self.settings.reduce_motion {
                None
            } else {
                Some(Transition::start(kind))
            };
            self.invalidate_largest();
            self.schedule_largest();
        }
    }

    pub(super) fn set_view(&mut self, mode: ViewMode) {
        if self.view != mode {
            self.view = mode;
            self.geom = None;
            self.transition = if self.settings.reduce_motion {
                None
            } else {
                Some(Transition::start(TransKind::ViewSwitch))
            };
        }
    }

    pub(super) fn copy_focus_path(&mut self) {
        let Some(tree) = &self.tree else { return };
        let id = self.hovered.or(self.selected).unwrap_or(self.focus);
        let path = tree.path_string(id);
        self.status = crate::tr!("Copied {}", path);
        self.clipboard_set(&path);
    }

    pub(super) fn clipboard_set(&self, text: &str) {
        // egui context clipboard is set from update; stash via arboard-less cmd on macOS.
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("pbcopy")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut c| {
                    use std::io::Write;
                    if let Some(mut stdin) = c.stdin.take() {
                        stdin.write_all(text.as_bytes())?;
                    }
                    c.wait()?;
                    Ok(())
                });
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = text;
        }
    }

    pub(super) fn percent_of_parent(&self, id: NodeId) -> Option<f32> {
        let tree = self.tree.as_ref()?;
        let node = tree.try_node(id)?;
        let parent = node.parent?;
        let psize = tree.try_node(parent)?.size(self.size_mode).max(1);
        Some((node.size(self.size_mode) as f32 / psize as f32) * 100.0)
    }

    pub(super) fn apply_filter_debounce(&mut self) {
        let ms = self.settings.flags.debounce_filter_ms;
        if let Some(at) = self.filter_edited_at {
            if at.elapsed() >= Duration::from_millis(ms) {
                if self.filter.query != self.filter_draft {
                    self.filter.query = self.filter_draft.clone();
                    self.filter.rebuild_regex();
                    self.geom = None;
                }
                self.filter_edited_at = None;
            }
        }
    }

    pub(super) fn persist_settings_from_viewport(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            if let Some(rect) = i.viewport().inner_rect {
                self.settings.window_width = rect.width();
                self.settings.window_height = rect.height();
            }
        });
        self.settings.default_view = self.view;
        self.settings.default_size_mode = self.size_mode;
    }

    pub(super) fn go_up(&mut self) {
        if let Some(tree) = &self.tree {
            if let Some(parent) = tree.try_node(self.focus).and_then(|n| n.parent) {
                self.navigate(parent, true);
            }
        }
    }

    pub(super) fn go_back(&mut self) {
        if let Some(prev) = self.history_back.pop() {
            self.history_fwd.push(self.focus);
            self.focus = prev;
            self.clear_selection();
            self.invalidate_largest();
            self.schedule_largest();
        }
    }

    pub(super) fn go_forward(&mut self) {
        if let Some(next) = self.history_fwd.pop() {
            self.history_back.push(self.focus);
            self.focus = next;
            self.clear_selection();
            self.invalidate_largest();
            self.schedule_largest();
        }
    }

    pub(super) fn reset_tree_view_state(&mut self) {
        self.clear_selection();
        self.hovered = None;
        self.expanded.clear();
        self.tree_rows.clear();
        self.tree_revealed = None;
        self.tree_scroll = false;
        self.geom = None;
        self.caches.bump_tree_epoch();
    }

    pub(super) fn select_single(&mut self, id: NodeId) {
        self.selected = Some(id);
        self.selected_set.clear();
        self.selected_set.push(id);
        self.selection_anchor = Some(id);
    }

    pub(super) fn clear_selection(&mut self) {
        self.selected = None;
        self.selected_set.clear();
        self.selection_anchor = None;
    }

    pub(super) fn toggle_select_node(&mut self, id: NodeId) {
        if let Some(pos) = self.selected_set.iter().position(|&x| x == id) {
            self.selected_set.remove(pos);
            if self.selected == Some(id) {
                self.selected = self.selected_set.last().copied();
            }
        } else {
            self.selected_set.push(id);
            self.selected = Some(id);
        }
        self.selection_anchor = self.selected;
    }

    pub(super) fn select_range_in_rows(&mut self, to: NodeId) {
        let anchor = self.selection_anchor.or(self.selected);
        if let Some(a) = anchor {
            if let (Some(i0), Some(i1)) = (
                self.tree_rows.iter().position(|&x| x == a),
                self.tree_rows.iter().position(|&x| x == to),
            ) {
                let (lo, hi) = if i0 <= i1 { (i0, i1) } else { (i1, i0) };
                self.selected_set = self.tree_rows[lo..=hi].to_vec();
                self.selected = Some(to);
                return;
            }
        }
        self.select_single(to);
    }

    pub(super) fn reset_filters(&mut self) {
        self.filter.clear();
        self.filter_draft.clear();
        self.ext_draft.clear();
        self.min_mb_draft.clear();
        self.max_mb_draft.clear();
        self.filter_edited_at = None;
        self.geom = None;
    }

    pub(super) fn cycle_map_child(&mut self, forward: bool) {
        let Some(tree) = &self.tree else { return };
        let kids: Vec<NodeId> = if !self.filter.is_active() {
            tree.children(self.focus).to_vec()
        } else {
            tree.children(self.focus)
                .iter()
                .copied()
                .filter(|&cid| {
                    let n = tree.node(cid);
                    self.filter
                        .matches(&n.name, n.is_dir, n.size(self.size_mode))
                })
                .collect()
        };
        if kids.is_empty() {
            return;
        }
        let next = match self
            .selected
            .and_then(|s| kids.iter().position(|&c| c == s))
        {
            Some(i) if forward => kids[(i + 1) % kids.len()],
            Some(i) => kids[(i + kids.len() - 1) % kids.len()],
            None if forward => kids[0],
            None => kids[kids.len() - 1],
        };
        self.select_single(next);
    }

    pub(super) fn restore_rescan_tree(
        &mut self,
        mut tree: Arc<SpaceTree>,
        saved: RescanSavedPaths,
    ) {
        if self.size_mode != SizeMode::OnDisk {
            Arc::make_mut(&mut tree).sort_children_by(self.size_mode);
        }
        self.focus = find_path(&tree, &saved.focus).unwrap_or(tree.root);
        self.selected = saved.selected.as_deref().and_then(|p| find_path(&tree, p));
        self.selected_set = self.selected.into_iter().collect();
        self.selection_anchor = self.selected;
        self.expanded = saved
            .expanded
            .iter()
            .filter_map(|p| find_path(&tree, p))
            .collect();
        self.review.clear();
        for p in &saved.review {
            if let Some(id) = find_path(&tree, p) {
                if tree
                    .try_node(id)
                    .is_some_and(|n| !n.synthetic && n.parent.is_some())
                {
                    self.review.push(id);
                }
            }
        }
        self.history_back = saved
            .history_back
            .iter()
            .filter_map(|p| find_path(&tree, p))
            .filter(|&id| id != self.focus)
            .collect();
        self.history_fwd = saved
            .history_fwd
            .iter()
            .filter_map(|p| find_path(&tree, p))
            .filter(|&id| id != self.focus)
            .collect();
        self.hovered = None;
        self.tree_rows.clear();
        self.tree_revealed = None;
        self.tree_scroll = false;
        self.geom = None;
        self.cache_stale = false;
        self.from_cache = false;
        self.status = crate::tr!(
            "Rescanned · {} items · {}",
            tree.item_count,
            crate::i18n::bytes(tree.total_size_mode(self.size_mode))
        );
        self.caches.invalidate_all();
        self.tree = Some(tree);
        self.invalidate_largest();
        self.schedule_largest();
        if let Some(tree) = self.tree.clone() {
            self.save_cache_async(tree);
        }
    }

    /// A rule path (may start with `~` or contain a glob): select it in Folders
    /// when the open scan indexed it, otherwise reveal it in Finder.
    pub(super) fn reveal_rule_path(&mut self, raw: &str) {
        let mut path = raw.to_owned();
        if let (Some(rest), Some(home)) = (path.strip_prefix('~'), std::env::var_os("HOME")) {
            path = format!("{}{rest}", home.to_string_lossy());
        }
        // Stop at the first glob component: that folder is what exists.
        let path: PathBuf = PathBuf::from(&path)
            .components()
            .take_while(|c| !c.as_os_str().to_string_lossy().contains(['*', '?', '[']))
            .collect();
        if let Some(id) = self.tree.as_ref().and_then(|t| find_path(t, &path)) {
            let parent = self
                .tree
                .as_ref()
                .and_then(|t| t.try_node(id))
                .and_then(|n| n.parent);
            if let Some(parent) = parent {
                self.navigate(parent, true);
            }
            self.select_single(id);
            self.rules.open = false;
            self.choosing_target = false;
            self.mode = Mode::Explore;
            return;
        }
        crate::widgets::reveal_in_os(&path);
        self.status = crate::tr!("Not in this scan — opened {} in Finder", path.display());
    }

    pub(super) fn reveal_selected(&self) {
        let Some(tree) = &self.tree else { return };
        let id = self.selected.unwrap_or(self.focus);
        let path = tree.path_string(id);
        crate::widgets::reveal_in_os(std::path::Path::new(&path));
    }

    pub(super) fn quick_look_selected(&self) {
        let Some(tree) = &self.tree else { return };
        let id = self.selected.or(self.hovered).unwrap_or(self.focus);
        if let Some(n) = tree.try_node(id) {
            if !n.synthetic {
                let path = tree.path_string(id);
                crate::widgets::quick_look(std::path::Path::new(&path));
            }
        }
    }

    pub(super) fn open_terminal_selected(&self) {
        let Some(tree) = &self.tree else { return };
        let id = self.selected.or(self.hovered).unwrap_or(self.focus);
        if let Some(n) = tree.try_node(id) {
            if !n.synthetic {
                let path = tree.path_string(id);
                crate::widgets::open_in_terminal(std::path::Path::new(&path));
            }
        }
    }

    pub(super) fn toggle_selected_in_basket(&mut self) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let ids: Vec<NodeId> = if !self.selected_set.is_empty() {
            self.selected_set.clone()
        } else if let Some(id) = self.selected {
            vec![id]
        } else {
            return;
        };
        for id in ids {
            if tree
                .try_node(id)
                .is_some_and(|n| !n.synthetic && n.parent.is_some())
            {
                review::toggle_in_tree(&tree, &mut self.review, id);
            }
        }
    }

    pub(super) fn open_goto_dialog(&mut self) {
        self.show_goto = true;
        self.focus_goto = true;
        if self.goto_draft.is_empty() {
            if let Some(tree) = &self.tree {
                self.goto_draft = tree.path_string(self.focus);
            }
        }
    }

    pub(super) fn any_modal_open(&self) -> bool {
        self.confirm_delete
            || self.confirm_tm_clean
            || self.show_settings
            || self.show_about
            || self.show_shortcuts
            || self.show_goto
            || self.rules.open
            || self.show_tm_panel
    }

    pub(super) fn open_tm_panel(&mut self) {
        self.show_tm_panel = true;
        self.refresh_snapshots();
    }

    pub(super) fn open_rules_panel(&mut self) {
        self.rules.open = true;
        self.rules.error = None;
        self.start_rule_mgr_load();
    }

    pub(super) fn open_junk_panel(&mut self) {
        self.mode = Mode::Clean;
        self.start_quick_clean_scan();
    }

    pub(crate) fn start_quick_clean_scan(&mut self) {
        self.junk.open_and_scan();
        self.start_junk_scan();
        self.start_downloads_scan();
    }

    pub(super) fn set_mode(&mut self, mode: Mode) {
        if mode == Mode::Clean {
            self.junk.open = true;
            if self.junk.report().is_none() && !self.junk.loading {
                self.start_quick_clean_scan();
            }
        }
        self.mode = mode;
    }

    pub(super) fn review_bytes(&self) -> u64 {
        self.tree
            .as_ref()
            .map(|t| {
                self.caches
                    .review_sets(t, &self.review, self.size_mode)
                    .bytes
            })
            .unwrap_or(0)
    }

    /// Unified Trash queue combining Quick clean, Downloads, and Folders selections.
    pub(crate) fn basket_items(&self) -> Vec<BasketItem> {
        self.caches.basket_items(
            self.tree.as_ref(),
            self.size_mode,
            &self.review,
            &self.junk,
            &self.downloads,
        )
    }

    pub(crate) fn basket_len(&self) -> usize {
        self.caches.basket_len(
            self.tree.as_ref(),
            self.size_mode,
            &self.review,
            &self.junk,
            &self.downloads,
        )
    }

    pub(crate) fn basket_bytes(&self) -> u64 {
        self.caches.basket_bytes(
            self.tree.as_ref(),
            self.size_mode,
            &self.review,
            &self.junk,
            &self.downloads,
        )
    }

    /// "N not in this scan · K snapshots" when the volume holds more than the scan saw.
    pub(super) fn scan_gap_label(&self) -> Option<String> {
        let tree = self.tree.as_ref()?;
        let vol = volumes::match_volume(&self.volumes, &tree.root_path)?;
        if tree.root_path != vol.mount {
            return None;
        }
        let indexed = tree.total_size_mode(self.size_mode);
        let gap = vol.used().checked_sub(indexed.saturating_add(64 << 20))?;
        let snaps = if self.tm_snapshots.is_empty() {
            vol.snapshot_count as usize
        } else {
            self.tm_snapshots.len()
        };
        Some(if snaps > 0 {
            crate::tr!(
                "{} not in this scan · {} snapshots",
                crate::i18n::bytes(gap + (64 << 20)),
                snaps
            )
        } else {
            crate::tr!("{} not in this scan", crate::i18n::bytes(gap + (64 << 20)))
        })
    }

    pub(crate) fn remove_from_basket(&mut self, path: &std::path::Path) {
        let slice = [path.to_path_buf()];
        self.junk.exclude_paths(&slice);
        self.downloads.exclude_paths(&slice);
        if let Some(tree) = &self.tree {
            self.review
                .retain(|&id| !std::path::Path::new(&tree.path_string(id)).starts_with(path));
        }
    }

    pub(crate) fn is_direct_delete(&self) -> bool {
        matches!(self.delete_source, DeleteSource::Direct(_))
    }

    pub(crate) fn active_delete_items(&self) -> Vec<BasketItem> {
        match &self.delete_source {
            DeleteSource::Basket => self.basket_items(),
            DeleteSource::Direct(items) => items.clone(),
        }
    }

    pub(crate) fn remove_from_active_delete(&mut self, path: &std::path::Path) {
        match &mut self.delete_source {
            DeleteSource::Basket => self.remove_from_basket(path),
            DeleteSource::Direct(items) => items.retain(|item| item.path != path),
        }
    }

    pub(super) fn selected_deletable_nodes(&self) -> Vec<NodeId> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let mut ids = Vec::new();
        for &id in self.selected.iter().chain(self.selected_set.iter()) {
            if !ids.contains(&id)
                && is_live_node(tree, id)
                && tree
                    .try_node(id)
                    .is_some_and(|n| !n.synthetic && n.parent.is_some())
            {
                ids.push(id);
            }
        }
        ids
    }

    fn confirm_setting_for(&self, method: space_clean::DeleteMethod) -> bool {
        match method {
            space_clean::DeleteMethod::Trash => self.settings.confirm_trash,
            space_clean::DeleteMethod::Permanent => self.settings.confirm_permanent_delete,
        }
    }

    fn request_delete(&mut self, method: space_clean::DeleteMethod, source: DeleteSource) {
        if self.busy {
            return;
        }
        let empty = match &source {
            DeleteSource::Basket => self.basket_len() == 0,
            DeleteSource::Direct(items) => items.is_empty(),
        };
        if empty {
            return;
        }
        self.delete_method = method;
        self.delete_source = source;
        if self.confirm_setting_for(method) {
            self.confirm_delete = true;
        } else {
            self.apply_basket();
        }
    }

    pub(crate) fn request_trash(&mut self) {
        self.request_delete(space_clean::DeleteMethod::Trash, DeleteSource::Basket);
    }

    pub(crate) fn request_permanent_basket(&mut self) {
        self.request_delete(space_clean::DeleteMethod::Permanent, DeleteSource::Basket);
    }

    pub(crate) fn request_delete_nodes(
        &mut self,
        ids: &[NodeId],
        method: space_clean::DeleteMethod,
    ) {
        let Some(tree) = &self.tree else {
            return;
        };
        let raw: Vec<BasketItem> = ids
            .iter()
            .copied()
            .filter(|&id| {
                is_live_node(tree, id)
                    && tree
                        .try_node(id)
                        .is_some_and(|n| !n.synthetic && n.parent.is_some())
            })
            .filter_map(|id| {
                let node = tree.try_node(id)?;
                Some(BasketItem {
                    path: PathBuf::from(tree.path_string(id)),
                    bytes: node.size(self.size_mode),
                    is_dir: node.is_dir,
                    junk_guard: None,
                })
            })
            .collect();
        self.request_delete_direct(raw, method);
    }

    pub(crate) fn request_delete_direct(
        &mut self,
        items: Vec<BasketItem>,
        method: space_clean::DeleteMethod,
    ) {
        let mut coalesced = space_clean::coalesce_nested_paths(items, |item| item.path.as_path());
        coalesced.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
        self.request_delete(method, DeleteSource::Direct(coalesced));
    }

    pub(crate) fn basket_item_from_finding(finding: &space_clean::Finding) -> BasketItem {
        let is_dir = std::fs::symlink_metadata(&finding.path)
            .map(|m| m.is_dir())
            .unwrap_or(finding.is_dir);
        BasketItem {
            path: finding.path.clone(),
            bytes: finding.bytes,
            is_dir,
            junk_guard: Some((finding.root.clone(), finding.min_age_days)),
        }
    }

    pub(crate) fn clear_all_cache(&mut self) {
        if let Some(cache) = &self.cache {
            if let Ok(removed) = cache.wipe_all() {
                self.recent.clear();
                self.status = crate::tr!("Cleared {} cached scans", removed);
            }
        }
    }

    pub(crate) fn remove_cache_entry(&mut self, root: &std::path::Path) {
        if let Some(cache) = &self.cache {
            let _ = cache.invalidate(root);
            self.recent = cache.list_recent().unwrap_or_default();
        }
    }
}
