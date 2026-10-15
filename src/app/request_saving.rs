use super::{PoopmanApp, app_notice};
use crate::{collections_panel::CollectionTarget, request_tab::RequestSnapshot};
use gpui::*;
use gpui_component::{
    IndexPath, WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    input::{Input, InputState},
    select::{Select, SelectState},
    v_flex,
};

#[cfg(test)]
mod tests;

#[derive(Clone)]
struct PendingSave {
    tab_id: usize,
    saved_id: Option<i64>,
    snapshot: RequestSnapshot,
    close_after: bool,
}

impl PoopmanApp {
    pub(super) fn save_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_active_request(false, window, cx);
    }

    pub(super) fn save_active_request(
        &mut self,
        save_as: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.request_tabs.get(self.active_tab_index) else {
            return;
        };
        self.save_tab(tab.id, save_as, false, window, cx);
    }

    fn save_tab(
        &mut self,
        tab_id: usize,
        save_as: bool,
        close_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) || self.saving_tabs.contains(&tab_id) {
            return;
        }
        self.save_current_tab_state(cx);
        let Some(tab) = self.request_tabs.iter().find(|tab| tab.id == tab_id) else {
            return;
        };
        let mut save = PendingSave {
            tab_id,
            saved_id: if save_as { None } else { tab.saved_request_id },
            snapshot: tab.snapshot(),
            close_after,
        };
        if save.saved_id.is_some() {
            self.persist_saved_request(save, window, cx);
            return;
        }
        let dialog_title = if save_as {
            "Save request as"
        } else {
            "Save request"
        };
        save.snapshot.saved_name = Some(
            if tab.title.trim().is_empty() || tab.title == "New Request" {
                format!("{} request", tab.request.method.as_str())
            } else {
                tab.title.clone()
            },
        );
        let targets = self.collections_panel.read(cx).request_targets();
        if !targets.is_empty() {
            self.open_request_save_dialog(save, dialog_title, targets, window, cx);
            return;
        }
        self.saving_tabs.insert(tab_id);
        let db = self.db.clone();
        let task = cx.background_spawn(async move { db.create_collection("My Collection") });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.saving_tabs.remove(&tab_id);
                match result {
                    Ok(collection_id) => {
                        this.collections_panel
                            .update(cx, |panel, cx| panel.reload(window, cx));
                        if !window.has_active_dialog(cx)
                            && this.request_tabs.iter().any(|tab| tab.id == tab_id)
                        {
                            this.open_request_save_dialog(
                                save,
                                dialog_title,
                                vec![CollectionTarget {
                                    collection_id,
                                    folder_id: None,
                                    label: "My Collection".into(),
                                }],
                                window,
                                cx,
                            );
                        }
                    }
                    Err(error) => app_notice(window, cx, "Save failed", error.to_string()),
                }
            })?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn open_request_save_dialog(
        &mut self,
        save: PendingSave,
        title: &'static str,
        targets: Vec<CollectionTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Request name");
            input.set_value(
                save.snapshot.saved_name.clone().unwrap_or_default(),
                window,
                cx,
            );
            input
        });
        let selected = self.collections_panel.read(cx).selected_target();
        let selected_index = targets
            .iter()
            .position(|target| {
                if let Some(collection_id) = save.snapshot.collection_id {
                    target.collection_id == collection_id
                        && target.folder_id == save.snapshot.folder_id
                } else {
                    selected.as_ref().is_some_and(|selected| selected == target)
                }
            })
            .unwrap_or(0);
        let labels = targets
            .iter()
            .map(|target| target.label.clone())
            .collect::<Vec<_>>();
        let target_select = cx.new(|cx| {
            SelectState::new(
                labels,
                Some(IndexPath::default().row(selected_index)),
                window,
                cx,
            )
        });
        let viewport = window.viewport_size();
        let width = px(520.).min((viewport.width - px(32.)).max(px(280.)));
        let height = (viewport.height - px(80.)).max(px(180.));
        let app = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let name_input_for_ok = name_input.clone();
            let target_select_for_ok = target_select.clone();
            let targets = targets.clone();
            let save = save.clone();
            let app = app.clone();
            dialog
                .title(div().text_lg().font_weight(FontWeight::BOLD).child(title))
                .w(width)
                .max_h(height)
                .margin_top(px(16.))
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().child("Request name"))
                                .child(Input::new(&name_input)),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().child("Collection / folder"))
                                .child(Select::new(&target_select)),
                        ),
                )
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Save"))
                .on_ok(move |_, window, cx| {
                    let name = name_input_for_ok.read(cx).value().trim().to_string();
                    if name.is_empty() {
                        return false;
                    }
                    let index = target_select_for_ok
                        .read(cx)
                        .selected_index(cx)
                        .map(|index| index.row)
                        .unwrap_or(0);
                    let Some(target) = targets.get(index) else {
                        return false;
                    };
                    let mut save = save.clone();
                    save.snapshot.saved_name = Some(name);
                    save.snapshot.collection_id = Some(target.collection_id);
                    save.snapshot.folder_id = target.folder_id;
                    app.update(cx, |app, cx| app.persist_saved_request(save, window, cx));
                    true
                })
        });
    }

    fn persist_saved_request(
        &mut self,
        save: PendingSave,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = save.snapshot.clone();
        let (Some(collection_id), Some(name)) = (snapshot.collection_id, snapshot.saved_name)
        else {
            app_notice(
                window,
                cx,
                "Save failed",
                "Choose a collection and a request name before saving.",
            );
            return;
        };
        if !self.saving_tabs.insert(save.tab_id) {
            return;
        }
        let db = self.db.clone();
        let saved_id = save.saved_id;
        let task = cx.background_spawn(async move {
            db.save_request_draft(
                saved_id,
                collection_id,
                snapshot.folder_id,
                &name,
                &snapshot.request,
                &snapshot.params_state,
                &snapshot.headers_state,
                Some(&snapshot.body_draft),
            )
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.saving_tabs.remove(&save.tab_id);
                match result {
                    Ok(saved_id) => {
                        this.save_current_tab_state(cx);
                        if let Some(index) = this
                            .request_tabs
                            .iter()
                            .position(|tab| tab.id == save.tab_id)
                        {
                            let tab = &mut this.request_tabs[index];
                            tab.saved_request_id = Some(saved_id);
                            if save.saved_id.is_none() {
                                tab.collection_id = save.snapshot.collection_id;
                                tab.folder_id = save.snapshot.folder_id;
                                tab.saved_name = save.snapshot.saved_name.clone();
                                tab.update_title_from_saved_name();
                            }
                            tab.mark_saved(save.snapshot);
                            if save.close_after && !tab.is_dirty() {
                                this.close_tab_without_prompt(index, window, cx);
                            }
                        }
                        this.refresh_saved_tab_ui(cx);
                        this.collections_panel
                            .update(cx, |panel, cx| panel.reload(window, cx));
                    }
                    Err(error) => app_notice(window, cx, "Save failed", error.to_string()),
                }
            })?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn prompt_close_tab(
        &mut self,
        tab_id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.request_tabs.iter().find(|tab| tab.id == tab_id) else {
            return;
        };
        let title = tab.title.clone();
        let app = cx.entity();
        let viewport = window.viewport_size();
        let width = px(460.).min((viewport.width - px(32.)).max(px(280.)));
        let height = (viewport.height - px(80.)).max(px(180.));
        window.open_dialog(cx, move |dialog, _, _| {
            let save_app = app.clone();
            let discard_app = app.clone();
            dialog
                .title(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .child("Unsaved changes"),
                )
                .w(width)
                .max_h(height)
                .margin_top(px(16.))
                .child(format!("Save changes to ‘{title}’ before closing?"))
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Save"))
                .footer(move |ok, cancel, window, cx| {
                    let app = discard_app.clone();
                    vec![
                        cancel(window, cx),
                        div()
                            .debug_selector(|| "discard-request-changes".into())
                            .child(
                                Button::new("discard-request-changes")
                                    .label("Discard")
                                    .danger()
                                    .on_click(move |_, window, cx| {
                                        window.close_dialog(cx);
                                        app.update(cx, |app, cx| {
                                            if let Some(index) = app
                                                .request_tabs
                                                .iter()
                                                .position(|tab| tab.id == tab_id)
                                            {
                                                app.close_tab_without_prompt(index, window, cx);
                                            }
                                        });
                                    }),
                            )
                            .into_any_element(),
                        ok(window, cx),
                    ]
                })
                .on_ok(move |_, window, cx| {
                    // Close only this confirmation; a first-save dialog may open next.
                    window.close_dialog(cx);
                    save_app.update(cx, |app, cx| app.save_tab(tab_id, false, true, window, cx));
                    false
                })
        });
    }
}
