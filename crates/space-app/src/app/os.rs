//! Quick Look, Open Terminal here, Share and the typed path field (⌘L).

use std::path::PathBuf;

use space_core::NodeId;

use super::{find_path, ui, SpaceApp};
use crate::os_actions::{self, Outcome};

/// An OS hand-off for one node, asked for from a menu or the details pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OsVerb {
    QuickLook,
    Terminal,
    Share,
}

impl SpaceApp {
    /// Open the editable path field in the toolbar with the folder in view.
    pub(super) fn begin_path_edit(&mut self) {
        let Some(tree) = &self.tree else { return };
        self.path_edit = Some(tree.path_string(self.focus));
        self.path_edit_focus = true;
    }

    /// Go to what was typed: a folder or file in this scan, or offer to
    /// scan a folder outside it.
    pub(super) fn submit_path_edit(&mut self, raw: &str) {
        let Some(tree) = self.tree.clone() else { return };
        let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf());
        let path = os_actions::expand_typed_path(raw, home.as_deref());
        if path.as_os_str().is_empty() {
            return;
        }
        if let Some(id) = find_path(&tree, &path) {
            let n = tree.node(id);
            if n.is_dir {
                self.navigate(id, true);
            } else if let Some(parent) = n.parent {
                self.navigate(parent, true);
                self.select_one(Some(id));
                self.tree_scroll = true;
            }
            return;
        }
        if path.is_dir() {
            self.toasts.push(
                ui::ToastLevel::Info,
                crate::tr!("{} is not in this scan", path.display()),
                Some(ui::ToastAction::ScanPath(path)),
            );
        } else {
            self.notify(
                ui::ToastLevel::Error,
                crate::tr!("Folder not found: {}", path.display()),
            );
        }
    }

    fn node_path(&self, id: NodeId) -> Option<PathBuf> {
        let tree = self.tree.as_ref()?;
        let n = tree.try_node(id)?;
        (!n.synthetic).then(|| PathBuf::from(tree.path_string(id)))
    }

    /// Space: Quick Look on macOS; elsewhere open a file with its default app.
    pub(super) fn quick_look_selected(&mut self) {
        if let Some(id) = self.selected {
            self.os_action(id, OsVerb::QuickLook);
        }
    }

    pub(crate) fn os_action(&mut self, id: NodeId, verb: OsVerb) {
        let Some(path) = self.node_path(id) else {
            return;
        };
        let outcome = match verb {
            OsVerb::QuickLook => match os_actions::quick_look(&path) {
                Outcome::Unsupported if path.is_file() => {
                    let out = os_actions::open_default(&path);
                    if out == Outcome::Done {
                        self.notify(
                            ui::ToastLevel::Info,
                            crate::tr!("Opened {}", path.display()),
                        );
                    }
                    out
                }
                other => other,
            },
            OsVerb::Terminal => os_actions::open_terminal(&path),
            OsVerb::Share => {
                let out = os_actions::share(&path);
                self.pending_copy = Some(path.to_string_lossy().into_owned());
                self.notify(
                    ui::ToastLevel::Info,
                    crate::i18n::t("Path copied and shown in Finder — share it from there").to_string(),
                );
                out
            }
        };
        match outcome {
            Outcome::Done => {}
            Outcome::Unsupported => self.notify(
                ui::ToastLevel::Info,
                crate::i18n::t("Quick Look is available on macOS; folders open with Enter").to_string(),
            ),
            Outcome::Failed(e) => self.notify(
                ui::ToastLevel::Error,
                crate::tr!("Could not open it: {}", e),
            ),
        }
    }
}
