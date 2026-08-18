//! Collections sidebar.
//!
//! The panel owns only the tree presentation and collection-level actions. A
//! request is opened by emitting `SavedRequestClicked`; the app remains the
//! owner of tabs and the request editor.

use anyhow::Result;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, Sizable as _, WindowExt,
    button::*,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    v_flex,
};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use crate::db::Database;
use crate::postman::{self, ImportResult};
use crate::theme::method_color;
use crate::types::{Collection, CollectionFolder, SavedRequest};

#[derive(Clone)]
pub struct SavedRequestClicked {
    pub request: SavedRequest,
}

#[derive(Clone)]
pub struct NewRequestRequested {
    pub target: CollectionTarget,
}

#[derive(Clone, Default)]
pub struct CollectionsChanged {
    /// Requests removed by a collection/folder/request deletion. Tabs pointing
    /// at these rows must become ordinary unsaved tabs.
    pub deleted_request_ids: Vec<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeRef {
    Collection(i64),
    Folder(i64),
    Request(i64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionTarget {
    pub collection_id: i64,
    pub folder_id: Option<i64>,
    pub label: String,
}

#[derive(Clone, Debug)]
enum NameAction {
    CreateCollection,
    CreateFolder {
        collection_id: i64,
        parent_id: Option<i64>,
    },
    RenameCollection(i64),
    RenameFolder(i64),
    RenameRequest(i64),
}

pub struct CollectionsPanel {
    db: Arc<Database>,
    collections: Vec<Collection>,
    selected: Option<NodeRef>,
    expanded_collections: HashSet<i64>,
    expanded_folders: HashSet<i64>,
    search: Entity<InputState>,
    query: String,
    reload_generation: u64,
    list_scroll_handle: ScrollHandle,
}

impl EventEmitter<SavedRequestClicked> for CollectionsPanel {}
impl EventEmitter<NewRequestRequested> for CollectionsPanel {}
impl EventEmitter<CollectionsChanged> for CollectionsPanel {}

impl CollectionsPanel {
    pub fn new(
        db: Arc<Database>,
        collections: Vec<Collection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let expanded_collections = collections.iter().map(|collection| collection.id).collect();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search collections"));
        cx.subscribe(&search, Self::on_search_change).detach();

        Self {
            db,
            collections,
            selected: None,
            expanded_collections,
            expanded_folders: HashSet::new(),
            search,
            query: String::new(),
            reload_generation: 0,
            list_scroll_handle: ScrollHandle::new(),
        }
    }

    #[cfg_attr(feature = "profile", profiling::function)]
    pub fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_generation = self.reload_generation.wrapping_add(1);
        let generation = self.reload_generation;
        let db = self.db.clone();
        let task = cx.background_spawn(async move { db.load_collections() });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.reload_generation != generation {
                    return;
                }
                match result {
                    Ok(collections) => this.apply_loaded_collections(collections, cx),
                    Err(error) => log::error!("Failed to reload collections: {}", error),
                }
            })?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn apply_loaded_collections(&mut self, collections: Vec<Collection>, cx: &mut Context<Self>) {
        self.collections = collections;
        let collection_ids: HashSet<i64> = self.collections.iter().map(|c| c.id).collect();
        self.expanded_collections
            .retain(|id| collection_ids.contains(id));
        if let Some(selected) = self.selected
            && !self.node_exists(selected)
        {
            self.selected = None;
        }
        cx.notify();
    }

    pub fn selected_target(&self) -> Option<CollectionTarget> {
        match self.selected? {
            NodeRef::Collection(collection_id) => self
                .collections
                .iter()
                .find(|collection| collection.id == collection_id)
                .map(|collection| CollectionTarget {
                    collection_id,
                    folder_id: None,
                    label: collection.name.clone(),
                }),
            NodeRef::Folder(folder_id) => {
                self.find_folder(folder_id).map(|folder| CollectionTarget {
                    collection_id: folder.collection_id,
                    folder_id: Some(folder.id),
                    label: self.folder_label(folder.id),
                })
            }
            NodeRef::Request(request_id) => self.find_request(request_id).map(|request| {
                let label = if let Some(folder_id) = request.folder_id {
                    self.folder_label(folder_id)
                } else {
                    self.collections
                        .iter()
                        .find(|collection| collection.id == request.collection_id)
                        .map(|collection| collection.name.clone())
                        .unwrap_or_else(|| "Collection".to_string())
                };
                CollectionTarget {
                    collection_id: request.collection_id,
                    folder_id: request.folder_id,
                    label,
                }
            }),
        }
    }

    pub fn select_target(&mut self, target: &CollectionTarget, cx: &mut Context<Self>) {
        self.selected = Some(match target.folder_id {
            Some(folder_id) => NodeRef::Folder(folder_id),
            None => NodeRef::Collection(target.collection_id),
        });
        cx.notify();
    }

    /// Return every valid save destination in display order. The first entry
    /// for a collection is its root; folders follow recursively.
    pub fn request_targets(&self) -> Vec<CollectionTarget> {
        let mut targets = Vec::new();
        for collection in &self.collections {
            targets.push(CollectionTarget {
                collection_id: collection.id,
                folder_id: None,
                label: collection.name.clone(),
            });
            self.append_folder_targets(&mut targets, collection, &collection.folders, "");
        }
        targets
    }

    fn append_folder_targets(
        &self,
        targets: &mut Vec<CollectionTarget>,
        collection: &Collection,
        folders: &[CollectionFolder],
        prefix: &str,
    ) {
        for folder in folders {
            let label = if prefix.is_empty() {
                format!("{} / {}", collection.name, folder.name)
            } else {
                format!("{} / {}", prefix, folder.name)
            };
            targets.push(CollectionTarget {
                collection_id: collection.id,
                folder_id: Some(folder.id),
                label: label.clone(),
            });
            self.append_folder_targets(targets, collection, &folder.folders, &label);
        }
    }

    fn on_search_change(
        &mut self,
        _state: Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            self.query = self.search.read(cx).value().to_string();
            cx.notify();
        }
    }

    fn node_exists(&self, node: NodeRef) -> bool {
        match node {
            NodeRef::Collection(id) => self
                .collections
                .iter()
                .any(|collection| collection.id == id),
            NodeRef::Folder(id) => self.find_folder(id).is_some(),
            NodeRef::Request(id) => self.find_request(id).is_some(),
        }
    }

    fn find_folder(&self, id: i64) -> Option<&CollectionFolder> {
        fn walk(folders: &[CollectionFolder], id: i64) -> Option<&CollectionFolder> {
            for folder in folders {
                if folder.id == id {
                    return Some(folder);
                }
                if let Some(found) = walk(&folder.folders, id) {
                    return Some(found);
                }
            }
            None
        }
        self.collections
            .iter()
            .find_map(|collection| walk(&collection.folders, id))
    }

    fn find_request(&self, id: i64) -> Option<&SavedRequest> {
        fn walk(folders: &[CollectionFolder], id: i64) -> Option<&SavedRequest> {
            for folder in folders {
                if let Some(found) = folder.requests.iter().find(|request| request.id == id) {
                    return Some(found);
                }
                if let Some(found) = walk(&folder.folders, id) {
                    return Some(found);
                }
            }
            None
        }
        for collection in &self.collections {
            if let Some(request) = collection.requests.iter().find(|request| request.id == id) {
                return Some(request);
            }
            if let Some(request) = walk(&collection.folders, id) {
                return Some(request);
            }
        }
        None
    }

    fn folder_label(&self, id: i64) -> String {
        fn walk(folders: &[CollectionFolder], id: i64, prefix: &str) -> Option<String> {
            for folder in folders {
                let label = if prefix.is_empty() {
                    folder.name.clone()
                } else {
                    format!("{} / {}", prefix, folder.name)
                };
                if folder.id == id {
                    return Some(label);
                }
                if let Some(found) = walk(&folder.folders, id, &label) {
                    return Some(found);
                }
            }
            None
        }
        self.collections
            .iter()
            .find_map(|collection| walk(&collection.folders, id, &collection.name))
            .unwrap_or_else(|| "Collection".to_string())
    }

    fn select_node(&mut self, node: NodeRef, cx: &mut Context<Self>) {
        self.selected = Some(node);
        cx.notify();
    }

    fn toggle_collection(&mut self, id: i64, cx: &mut Context<Self>) {
        if !self.expanded_collections.insert(id) {
            self.expanded_collections.remove(&id);
        }
        cx.notify();
    }

    fn toggle_folder(&mut self, id: i64, cx: &mut Context<Self>) {
        if !self.expanded_folders.insert(id) {
            self.expanded_folders.remove(&id);
        }
        cx.notify();
    }

    fn open_request(&mut self, request: &SavedRequest, cx: &mut Context<Self>) {
        self.selected = Some(NodeRef::Request(request.id));
        cx.emit(SavedRequestClicked {
            request: request.clone(),
        });
        cx.notify();
    }

    fn prompt_name(
        &mut self,
        title: &str,
        initial: &str,
        action: NameAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let initial = initial.to_string();
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Name");
            input.set_value(initial, window, cx);
            input
        });
        let panel = cx.entity();
        let title = title.to_string();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let panel = panel.clone();
            let input_for_ok = input.clone();
            let action = action.clone();
            dialog
                .title(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .child(title.clone()),
                )
                .w(px(420.))
                .child(v_flex().gap_2().child(Input::new(&input)))
                .confirm()
                .on_ok(move |_, window, cx: &mut App| {
                    let name = input_for_ok.read(cx).value().trim().to_string();
                    if name.is_empty() {
                        return false;
                    }
                    panel.update(cx, |panel, cx| {
                        panel.apply_name_action(action.clone(), &name, window, cx);
                    });
                    true
                })
        });
    }

    fn apply_name_action(
        &mut self,
        action: NameAction,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let db = self.db.clone();
        let name = name.to_string();
        let action_for_ui = action.clone();
        let task = cx.background_spawn(async move {
            match action {
                NameAction::CreateCollection => db
                    .create_collection(&name)
                    .map(|id| Some(NodeRef::Collection(id))),
                NameAction::CreateFolder {
                    collection_id,
                    parent_id,
                } => db
                    .create_folder(collection_id, parent_id, &name)
                    .map(|id| Some(NodeRef::Folder(id))),
                NameAction::RenameCollection(id) => db.rename_collection(id, &name).map(|()| None),
                NameAction::RenameFolder(id) => db.rename_folder(id, &name).map(|()| None),
                NameAction::RenameRequest(id) => db.rename_saved_request(id, &name).map(|()| None),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            match task.await {
                Ok(selected) => {
                    this.update_in(cx, |this, window, cx| {
                        if let Some(selected) = selected {
                            this.selected = Some(selected);
                            if let NameAction::CreateFolder {
                                collection_id,
                                parent_id,
                            } = action_for_ui
                            {
                                this.expanded_collections.insert(collection_id);
                                if let Some(parent_id) = parent_id {
                                    this.expanded_folders.insert(parent_id);
                                }
                            }
                        }
                        this.reload(window, cx);
                        cx.emit(CollectionsChanged::default());
                    })?;
                }
                Err(error) => log::error!("Failed to update collection tree: {}", error),
            }
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn request_ids_in_folder(folder: &CollectionFolder, ids: &mut Vec<i64>) {
        ids.extend(folder.requests.iter().map(|request| request.id));
        for child in &folder.folders {
            Self::request_ids_in_folder(child, ids);
        }
    }

    fn request_ids_in_collection(collection: &Collection) -> Vec<i64> {
        let mut ids = collection
            .requests
            .iter()
            .map(|request| request.id)
            .collect();
        for folder in &collection.folders {
            Self::request_ids_in_folder(folder, &mut ids);
        }
        ids
    }

    fn node_name(&self, node: NodeRef) -> String {
        match node {
            NodeRef::Collection(id) => self
                .collections
                .iter()
                .find(|collection| collection.id == id)
                .map(|collection| collection.name.clone())
                .unwrap_or_else(|| "Collection".to_string()),
            NodeRef::Folder(id) => self
                .find_folder(id)
                .map(|folder| folder.name.clone())
                .unwrap_or_else(|| "Folder".to_string()),
            NodeRef::Request(id) => self
                .find_request(id)
                .map(|request| request.name.clone())
                .unwrap_or_else(|| "Request".to_string()),
        }
    }

    fn prompt_delete(&mut self, node: NodeRef, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.node_name(node);
        let panel = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let panel = panel.clone();
            dialog
                .title(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .child("Delete item?"),
                )
                .w(px(440.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(format!(
                            "Delete ‘{}’? This also deletes nested requests.",
                            name
                        ))
                        .child(
                            div()
                                .text_sm()
                                .text_color(_cx.theme().muted_foreground)
                                .child("This cannot be undone."),
                        ),
                )
                .confirm()
                .on_ok(move |_, window, cx: &mut App| {
                    panel.update(cx, |panel, cx| {
                        panel.delete_node(node, window, cx);
                    });
                    true
                })
        });
    }

    fn delete_node(&mut self, node: NodeRef, window: &mut Window, cx: &mut Context<Self>) {
        let deleted_request_ids = match node {
            NodeRef::Collection(id) => self
                .collections
                .iter()
                .find(|collection| collection.id == id)
                .map(Self::request_ids_in_collection)
                .unwrap_or_default(),
            NodeRef::Folder(id) => self
                .find_folder(id)
                .map(|folder| {
                    let mut ids = Vec::new();
                    Self::request_ids_in_folder(folder, &mut ids);
                    ids
                })
                .unwrap_or_default(),
            NodeRef::Request(id) => vec![id],
        };
        let db = self.db.clone();
        let task = cx.background_spawn(async move {
            match node {
                NodeRef::Collection(id) => db.delete_collection(id),
                NodeRef::Folder(id) => db.delete_folder(id),
                NodeRef::Request(id) => db.delete_saved_request(id),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            match task.await {
                Ok(()) => {
                    this.update_in(cx, |this, window, cx| {
                        if this.selected == Some(node) {
                            this.selected = None;
                        }
                        this.reload(window, cx);
                        cx.emit(CollectionsChanged {
                            deleted_request_ids,
                        });
                    })?;
                }
                Err(error) => log::error!("Failed to delete collection item: {}", error),
            }
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn duplicate_node(&mut self, node: NodeRef, window: &mut Window, cx: &mut Context<Self>) {
        let db = self.db.clone();
        let task = cx.background_spawn(async move {
            match node {
                NodeRef::Collection(id) => db.duplicate_collection(id),
                NodeRef::Folder(id) => db.duplicate_folder(id),
                NodeRef::Request(id) => db.duplicate_saved_request(id),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            match task.await {
                Ok(id) => {
                    this.update_in(cx, |this, window, cx| {
                        this.selected = Some(match node {
                            NodeRef::Collection(_) => NodeRef::Collection(id),
                            NodeRef::Folder(_) => NodeRef::Folder(id),
                            NodeRef::Request(_) => NodeRef::Request(id),
                        });
                        this.reload(window, cx);
                        cx.emit(CollectionsChanged::default());
                    })?;
                }
                Err(error) => log::error!("Failed to duplicate collection item: {}", error),
            }
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    // The actual import implementation is kept separate from the file picker.
    // Both parsing and SQLite insertion run away from the UI thread.
    fn import_data(&mut self, imported: ImportResult, window: &mut Window, cx: &mut Context<Self>) {
        let base_name = if imported.collection.name.trim().is_empty() {
            "Imported Collection".to_string()
        } else {
            imported.collection.name.clone()
        };
        let name = if self
            .collections
            .iter()
            .any(|collection| collection.name == base_name)
        {
            format!("{} (Imported)", base_name)
        } else {
            base_name
        };
        let warnings = imported.warnings.clone();
        let db = self.db.clone();
        let name_for_db = name.clone();
        let task = cx.background_spawn(async move {
            db.insert_collection_tree(&imported.collection, &name_for_db)
        });
        cx.spawn_in(window, async move |this, cx| {
            match task.await {
                Ok(id) => {
                    this.update_in(cx, |this, window, cx| {
                        this.selected = Some(NodeRef::Collection(id));
                        this.expanded_collections.insert(id);
                        this.reload(window, cx);
                        cx.emit(CollectionsChanged::default());

                        if warnings.is_empty() {
                            panel_notice(
                                window,
                                cx,
                                "Collection imported",
                                format!("Imported {}.", name),
                            );
                        } else {
                            let details = warnings
                                .iter()
                                .take(8)
                                .map(|warning| format!("{}: {}", warning.path, warning.message))
                                .collect::<Vec<_>>()
                                .join("\n");
                            panel_notice(
                                window,
                                cx,
                                "Collection imported with warnings",
                                format!("Imported {}.\n\n{}", name, details),
                            );
                        }
                    })?;
                }
                Err(error) => {
                    cx.update(|window, cx| {
                        panel_notice(window, cx, "Import failed", error.to_string())
                    })?;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn start_import(&mut self, _event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import Postman collection".into()),
        });
        let panel = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            let paths = receiver.await??;
            let Some(path) = paths.and_then(|paths| paths.into_iter().next()) else {
                return Ok(());
            };
            let parse = cx.background_spawn(async move {
                let text = std::fs::read_to_string(path)?;
                Ok::<_, anyhow::Error>(postman::import_collection(&text)?)
            });
            match parse.await {
                Ok(imported) => {
                    panel.update_in(cx, |panel, window, cx| {
                        panel.import_data(imported, window, cx)
                    })?;
                }
                Err(error) => {
                    cx.update(|window, cx| {
                        panel_notice(window, cx, "Import failed", error.to_string())
                    })?;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn export_to_file(&mut self, collection_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(collection) = self
            .collections
            .iter()
            .find(|collection| collection.id == collection_id)
            .cloned()
        else {
            return;
        };
        let json = match postman::export_collection(&collection) {
            Ok(json) => json,
            Err(error) => {
                panel_notice(window, cx, "Export failed", error.to_string());
                return;
            }
        };
        let suggested = format!("{}.json", safe_filename(&collection.name));
        let receiver = cx.prompt_for_new_path(Path::new(""), Some(&suggested));
        cx.spawn_in(window, async move |_, cx| {
            let result: Result<()> = match receiver.await?? {
                Some(path) => {
                    cx.background_spawn(async move {
                        std::fs::write(path, json)?;
                        Ok::<_, anyhow::Error>(())
                    })
                    .await
                }
                None => Ok(()),
            };
            if let Err(error) = result {
                let _ = cx.update(|window, cx| {
                    panel_notice(window, cx, "Export failed", error.to_string())
                });
            }
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn render_collection(
        &self,
        collection: &Collection,
        owner: Entity<Self>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let query_matches =
            self.query.trim().is_empty() || collection_matches(collection, self.query.trim());
        if !query_matches {
            return div().into_any_element();
        }
        let id = collection.id;
        let selected = self.selected == Some(NodeRef::Collection(id));
        let expanded = self.expanded_collections.contains(&id) || !self.query.trim().is_empty();
        let mut children = Vec::new();
        if expanded {
            children.extend(
                collection
                    .requests
                    .iter()
                    .filter(|request| {
                        self.query.trim().is_empty()
                            || request_matches(request, self.query.trim())
                            || collection
                                .name
                                .to_ascii_lowercase()
                                .contains(&self.query.trim().to_ascii_lowercase())
                    })
                    .map(|request| self.render_request(request, 0, owner.clone(), cx)),
            );
            children.extend(
                collection
                    .folders
                    .iter()
                    .filter(|folder| {
                        self.query.trim().is_empty()
                            || folder_matches(folder, self.query.trim())
                            || collection
                                .name
                                .to_ascii_lowercase()
                                .contains(&self.query.trim().to_ascii_lowercase())
                    })
                    .map(|folder| self.render_folder(folder, 0, owner.clone(), cx)),
            );
        }
        let disclosure = if collection.folders.is_empty() && collection.requests.is_empty() {
            "·"
        } else if expanded {
            "▾"
        } else {
            "▸"
        };
        let row = h_flex()
            .id(("collection-row", id as u64))
            .w_full()
            .h(px(30.))
            .gap_1p5()
            .items_center()
            .px_2()
            .rounded(cx.theme().radius)
            .cursor_pointer()
            .when(selected, |row| row.bg(cx.theme().list_active))
            .hover(|style| style.bg(cx.theme().list_hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_node(NodeRef::Collection(id), cx);
            }))
            .child(
                div()
                    .id(("collection-toggle", id as u64))
                    .w(px(14.))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_collection(id, cx);
                    }))
                    .child(disclosure),
            )
            .child(div().text_sm().text_color(cx.theme().primary).child("▣"))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .font_weight(FontWeight::SEMIBOLD)
                    .overflow_x_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(collection.name.clone()),
            )
            .context_menu({
                let owner = owner.clone();
                move |menu, _window, _cx| collection_menu(menu, owner.clone(), id)
            });
        v_flex()
            .w_full()
            .child(row)
            .children(children)
            .into_any_element()
    }

    fn render_folder(
        &self,
        folder: &CollectionFolder,
        depth: usize,
        owner: Entity<Self>,
        cx: &Context<Self>,
    ) -> AnyElement {
        if !self.query.trim().is_empty() && !folder_matches(folder, self.query.trim()) {
            return div().into_any_element();
        }
        let id = folder.id;
        let selected = self.selected == Some(NodeRef::Folder(id));
        let expanded = self.expanded_folders.contains(&id) || !self.query.trim().is_empty();
        let mut children = Vec::new();
        if expanded {
            children.extend(
                folder
                    .requests
                    .iter()
                    .filter(|request| {
                        self.query.trim().is_empty() || request_matches(request, self.query.trim())
                    })
                    .map(|request| self.render_request(request, depth + 1, owner.clone(), cx)),
            );
            children.extend(
                folder
                    .folders
                    .iter()
                    .filter(|child| {
                        self.query.trim().is_empty() || folder_matches(child, self.query.trim())
                    })
                    .map(|child| self.render_folder(child, depth + 1, owner.clone(), cx)),
            );
        }
        let disclosure = if folder.folders.is_empty() && folder.requests.is_empty() {
            "·"
        } else if expanded {
            "▾"
        } else {
            "▸"
        };
        let row = h_flex()
            .id(("folder-row", id as u64))
            .w_full()
            .h(px(29.))
            .gap_1p5()
            .items_center()
            .pl(px(12. * depth as f32))
            .pr_2()
            .rounded(cx.theme().radius)
            .cursor_pointer()
            .when(selected, |row| row.bg(cx.theme().list_active))
            .hover(|style| style.bg(cx.theme().list_hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_node(NodeRef::Folder(id), cx);
            }))
            .child(
                div()
                    .id(("folder-toggle", id as u64))
                    .w(px(14.))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_folder(id, cx);
                    }))
                    .child(disclosure),
            )
            .child(div().text_sm().text_color(cx.theme().warning).child("▰"))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .overflow_x_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(folder.name.clone()),
            )
            .context_menu({
                let owner = owner.clone();
                let collection_id = folder.collection_id;
                move |menu, _window, _cx| folder_menu(menu, owner.clone(), id, collection_id)
            });
        v_flex()
            .w_full()
            .child(row)
            .children(children)
            .into_any_element()
    }

    fn render_request(
        &self,
        request: &SavedRequest,
        depth: usize,
        owner: Entity<Self>,
        cx: &Context<Self>,
    ) -> AnyElement {
        if !self.query.trim().is_empty() && !request_matches(request, self.query.trim()) {
            return div().into_any_element();
        }
        let id = request.id;
        let selected = self.selected == Some(NodeRef::Request(id));
        let method = request.request.method;
        let request_for_click = request.clone();
        let row = h_flex()
            .id(("saved-request-row", id as u64))
            .w_full()
            .h(px(29.))
            .gap_2()
            .items_center()
            .pl(px(28. + 12. * depth as f32))
            .pr_2()
            .rounded(cx.theme().radius)
            .cursor_pointer()
            .when(selected, |row| row.bg(cx.theme().list_active))
            .hover(|style| style.bg(cx.theme().list_hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_request(&request_for_click, cx);
            }))
            .child(
                div()
                    .w(px(42.))
                    .flex_shrink_0()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(method_color(method, cx.theme()))
                    .child(method.as_str()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .overflow_x_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(request.name.clone()),
            )
            .context_menu({
                let owner = owner.clone();
                move |menu, _window, _cx| request_menu(menu, owner.clone(), id)
            });
        row.into_any_element()
    }
}

impl Render for CollectionsPanel {
    #[cfg_attr(feature = "profile", profiling::function)]
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let owner = cx.entity();
        let query = self.query.trim();
        let visible = self
            .collections
            .iter()
            .any(|collection| query.is_empty() || collection_matches(collection, query));

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .items_center()
                    .w_full()
                    .gap_1()
                    .px_2()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child("Collections"),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.search)
                                .small()
                                .w_full()
                                .cleanable(true)
                                .prefix(Icon::empty().path("icons/search.svg")),
                        ),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_1()
                            .child(
                                Button::new("import-collection")
                                    .xsmall()
                                    .ghost()
                                    .label("Import")
                                    .tooltip("Import a Postman Collection v2.1 JSON")
                                    .on_click(cx.listener(Self::start_import)),
                            )
                            .child(
                                Button::new("new-collection")
                                    .xsmall()
                                    .ghost()
                                    .label("+")
                                    .tooltip("New collection")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.prompt_name(
                                            "New collection",
                                            "New Collection",
                                            NameAction::CreateCollection,
                                            window,
                                            cx,
                                        );
                                    })),
                            ),
                    ),
            )
            .when(!visible, |this| {
                let message = if query.is_empty() {
                    "No collections yet\n\nSave a request or create a collection to get started"
                } else {
                    "No collections match your search"
                };
                this.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .px_4()
                        .text_center()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(message),
                )
            })
            .when(visible, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .overflow_hidden()
                        .child(
                            v_flex()
                                .id("collections-tree-scroll")
                                .flex_1()
                                .w_full()
                                .min_h_0()
                                .gap_0p5()
                                .px_2()
                                .py_1()
                                .track_scroll(&self.list_scroll_handle)
                                .overflow_scroll()
                                .children(self.collections.iter().map(|collection| {
                                    self.render_collection(collection, owner.clone(), cx)
                                })),
                        )
                        .vertical_scrollbar(&self.list_scroll_handle),
                )
            })
    }
}

fn request_matches(request: &SavedRequest, query: &str) -> bool {
    let query = query.to_ascii_lowercase();
    request.name.to_ascii_lowercase().contains(&query)
        || request.request.url.to_ascii_lowercase().contains(&query)
        || request
            .request
            .method
            .as_str()
            .to_ascii_lowercase()
            .contains(&query)
}

fn folder_matches(folder: &CollectionFolder, query: &str) -> bool {
    folder
        .name
        .to_ascii_lowercase()
        .contains(&query.to_ascii_lowercase())
        || folder
            .requests
            .iter()
            .any(|request| request_matches(request, query))
        || folder
            .folders
            .iter()
            .any(|child| folder_matches(child, query))
}

fn collection_matches(collection: &Collection, query: &str) -> bool {
    let query = query.to_ascii_lowercase();
    collection.name.to_ascii_lowercase().contains(&query)
        || collection
            .requests
            .iter()
            .any(|request| request_matches(request, &query))
        || collection
            .folders
            .iter()
            .any(|folder| folder_matches(folder, &query))
}

fn collection_menu(
    menu: gpui_component::menu::PopupMenu,
    panel: Entity<CollectionsPanel>,
    id: i64,
) -> gpui_component::menu::PopupMenu {
    let new_request_panel = panel.clone();
    let new_folder_panel = panel.clone();
    let rename_panel = panel.clone();
    let duplicate_panel = panel.clone();
    let delete_panel = panel.clone();
    let export_panel = panel.clone();
    menu.item(PopupMenuItem::new("New request").on_click(move |_, _, cx| {
        new_request_panel.update(cx, |panel, cx| {
            let target = CollectionTarget {
                collection_id: id,
                folder_id: None,
                label: panel.node_name(NodeRef::Collection(id)),
            };
            panel.select_target(&target, cx);
            cx.emit(NewRequestRequested { target });
        });
    }))
    .item(
        PopupMenuItem::new("New folder").on_click(move |_, window, cx| {
            new_folder_panel.update(cx, |panel, cx| {
                panel.prompt_name(
                    "New folder",
                    "New Folder",
                    NameAction::CreateFolder {
                        collection_id: id,
                        parent_id: None,
                    },
                    window,
                    cx,
                );
            });
        }),
    )
    .item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
        rename_panel.update(cx, |panel, cx| {
            let current = panel.node_name(NodeRef::Collection(id));
            panel.prompt_name(
                "Rename collection",
                &current,
                NameAction::RenameCollection(id),
                window,
                cx,
            );
        });
    }))
    .item(
        PopupMenuItem::new("Duplicate").on_click(move |_, window, cx| {
            duplicate_panel.update(cx, |panel, cx| {
                panel.duplicate_node(NodeRef::Collection(id), window, cx)
            });
        }),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new("Export as Postman JSON").on_click(move |_, window, cx| {
            export_panel.update(cx, |panel, cx| panel.export_to_file(id, window, cx));
        }),
    )
    .item(PopupMenuItem::new("Delete").on_click(move |_, window, cx| {
        delete_panel.update(cx, |panel, cx| {
            panel.prompt_delete(NodeRef::Collection(id), window, cx)
        });
    }))
}

fn folder_menu(
    menu: gpui_component::menu::PopupMenu,
    panel: Entity<CollectionsPanel>,
    id: i64,
    collection_id: i64,
) -> gpui_component::menu::PopupMenu {
    let new_request_panel = panel.clone();
    let new_folder_panel = panel.clone();
    let rename_panel = panel.clone();
    let duplicate_panel = panel.clone();
    let delete_panel = panel.clone();
    menu.item(PopupMenuItem::new("New request").on_click(move |_, _, cx| {
        new_request_panel.update(cx, |panel, cx| {
            let target = CollectionTarget {
                collection_id,
                folder_id: Some(id),
                label: panel.folder_label(id),
            };
            panel.select_target(&target, cx);
            cx.emit(NewRequestRequested { target });
        });
    }))
    .item(
        PopupMenuItem::new("New subfolder").on_click(move |_, window, cx| {
            new_folder_panel.update(cx, |panel, cx| {
                panel.prompt_name(
                    "New subfolder",
                    "New Folder",
                    NameAction::CreateFolder {
                        collection_id,
                        parent_id: Some(id),
                    },
                    window,
                    cx,
                );
            });
        }),
    )
    .item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
        rename_panel.update(cx, |panel, cx| {
            let current = panel.node_name(NodeRef::Folder(id));
            panel.prompt_name(
                "Rename folder",
                &current,
                NameAction::RenameFolder(id),
                window,
                cx,
            );
        });
    }))
    .item(
        PopupMenuItem::new("Duplicate").on_click(move |_, window, cx| {
            duplicate_panel.update(cx, |panel, cx| {
                panel.duplicate_node(NodeRef::Folder(id), window, cx)
            });
        }),
    )
    .item(PopupMenuItem::new("Delete").on_click(move |_, window, cx| {
        delete_panel.update(cx, |panel, cx| {
            panel.prompt_delete(NodeRef::Folder(id), window, cx)
        });
    }))
}

fn request_menu(
    menu: gpui_component::menu::PopupMenu,
    panel: Entity<CollectionsPanel>,
    id: i64,
) -> gpui_component::menu::PopupMenu {
    let rename_panel = panel.clone();
    let duplicate_panel = panel.clone();
    let delete_panel = panel.clone();
    menu.item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
        rename_panel.update(cx, |panel, cx| {
            let current = panel.node_name(NodeRef::Request(id));
            panel.prompt_name(
                "Rename request",
                &current,
                NameAction::RenameRequest(id),
                window,
                cx,
            );
        });
    }))
    .item(
        PopupMenuItem::new("Duplicate").on_click(move |_, window, cx| {
            duplicate_panel.update(cx, |panel, cx| {
                panel.duplicate_node(NodeRef::Request(id), window, cx)
            });
        }),
    )
    .item(PopupMenuItem::new("Delete").on_click(move |_, window, cx| {
        delete_panel.update(cx, |panel, cx| {
            panel.prompt_delete(NodeRef::Request(id), window, cx)
        });
    }))
}

fn safe_filename(name: &str) -> String {
    let mut filename = name
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ch => ch,
        })
        .collect::<String>();
    if filename.trim().is_empty() {
        filename = "collection".to_string();
    }
    filename
}

fn panel_notice(
    window: &mut Window,
    cx: &mut App,
    title: impl Into<String>,
    message: impl Into<String>,
) {
    let title = title.into();
    let message = message.into();
    window.open_dialog(cx, move |dialog, _window, cx| {
        dialog
            .title(
                div()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .child(title.clone()),
            )
            .w(px(520.))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(message.clone()),
            )
            .alert()
    });
}
