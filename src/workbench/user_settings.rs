//! The window's side of the user settings: writing a change with VS Code's
//! refusals, telling about a settings file that does not parse, and
//! opening the file in an editor tab (specs `settings`, design 9).
use std::{path::Path, sync::Arc};

use gpui_kit::component::{
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::Notification,
};
use gpui_kit::*;
use serde_json::Value;

use super::{
    OpenSettingsEditor, Workbench,
    editor_group::TabKey,
    settings_editor::{SettingsEditor, SettingsEditorEvent},
};
use crate::{
    json_file::{self, real_path},
    settings::{SettingsStore, WriteError},
};

/// Marks the notification about a settings file that does not parse, so
/// the next one replaces it and a fixed file takes it away.
struct SettingsErrorNotice;

/// VS Code's `configurationEditing` wording.
const DIRTY: &str =
    "由于该文件具有未保存的更改，因此无法写入到用户设置。请先保存该用户设置文件，然后重试。";
const INVALID: &str = "无法写入用户设置。请打开用户设置并清除错误或警告，然后重试。";
const UNREADABLE: &str =
    "无法读取用户设置：settings.json 有语法错误。在修正之前，继续使用上一次读到的设置。";

impl Workbench {
    /// The tab key of `settings.json`.
    pub(super) fn settings_file_key(&self, cx: &App) -> Option<Arc<Path>> {
        let file = SettingsStore::global(cx).file()?;
        Some(real_path(file).into())
    }

    /// Writes `value` for the setting `id` and applies it, unless the file
    /// is open with unsaved edits or does not parse: then nothing changes
    /// and a notice says why, as VS Code does. Returns whether it was
    /// written.
    pub(super) fn write_setting(
        &mut self,
        id: &'static str,
        value: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let unsaved = self.settings_file_key(cx).filter(|key| {
            self.editors
                .buffers
                .get(key)
                .is_some_and(|open| open.buffer.read(cx).is_dirty(cx))
        });
        if let Some(key) = unsaved {
            self.refuse_dirty(key, id, value, window, cx);
            return false;
        }
        match cx.update_global(|store: &mut SettingsStore, _| store.update(id, value)) {
            Ok(()) => true,
            Err(WriteError::Invalid) => {
                let notice = Notification::error(INVALID).content(self.open_button());
                window.push_notification(notice, cx);
                false
            }
            Err(WriteError::Io(reason)) => {
                let notice = Notification::error(format!("无法写入用户设置：{reason}"));
                window.push_notification(notice.autohide(false), cx);
                false
            }
        }
    }

    /// 「保存并重试」 saves the open file, then writes the change again.
    fn refuse_dirty(
        &mut self,
        key: Arc<Path>,
        id: &'static str,
        value: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workbench = cx.entity().downgrade();
        let open = self.open_button();
        let notice = Notification::error(DIRTY).content(move |notice, window, cx| {
            let workbench = workbench.clone();
            let key = key.clone();
            let value = value.clone();
            let notification = cx.entity().downgrade();
            h_flex()
                .pt(px(8.))
                .gap(px(8.))
                .justify_end()
                .child(
                    Button::new("settings-save-and-retry")
                        .primary()
                        .label("保存并重试")
                        .on_click(move |_, window, cx| {
                            if let Some(notification) = notification.upgrade() {
                                notification.update(cx, |notification, cx| {
                                    notification.dismiss(window, cx)
                                });
                            }
                            let Some(workbench) = workbench.upgrade() else {
                                return;
                            };
                            let saved = workbench.update(cx, |workbench, cx| {
                                workbench.save_file(key.clone(), false, window, cx)
                            });
                            let value = value.clone();
                            let workbench = workbench.downgrade();
                            window
                                .spawn(cx, async move |cx| {
                                    if !saved.await {
                                        return;
                                    }
                                    let retried = workbench.update_in(cx, |workbench, window, cx| {
                                        workbench.write_setting(id, value, window, cx);
                                    });
                                    if let Err(error) = retried {
                                        log::warn!("view released before a setting was written: {error:#}");
                                    }
                                })
                                .detach();
                        }),
                )
                .child(open(notice, window, cx))
                .into_any_element()
        });
        window.push_notification(notice, cx);
    }

    /// The 「打开设置」 button of a settings notice, which opens the file.
    fn open_button(
        &self,
    ) -> impl Fn(&mut Notification, &mut Window, &mut Context<Notification>) -> AnyElement + 'static
    {
        |_, _, cx| {
            let notification = cx.entity().downgrade();
            h_flex()
                .pt(px(8.))
                .justify_end()
                .child(
                    Button::new("settings-open")
                        .label("打开设置")
                        .on_click(move |_, window, cx| {
                            if let Some(notification) = notification.upgrade() {
                                notification.update(cx, |notification, cx| {
                                    notification.dismiss(window, cx)
                                });
                            }
                            window.dispatch_action(Box::new(crate::OpenUserSettingsJson), cx);
                        }),
                )
                .into_any_element()
        }
    }

    /// ⌘,: the settings page, in its own tab, with the keyboard in its
    /// search box. There is one; opening it again goes to it.
    pub(super) fn open_settings(
        &mut self,
        _: &crate::OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = match &self.editors.settings {
            Some(open) => open.editor.clone(),
            None => {
                let editor = cx.new(|cx| SettingsEditor::new(window, cx));
                let subscription = cx.subscribe_in(
                    &editor,
                    window,
                    |this, _, event: &SettingsEditorEvent, window, cx| match event {
                        SettingsEditorEvent::Write { id, value } => {
                            this.write_setting(id, value.clone(), window, cx);
                        }
                    },
                );
                self.editors.settings = Some(OpenSettingsEditor {
                    editor: editor.clone(),
                    _subscription: subscription,
                });
                editor
            }
        };
        self.open_tab(TabKey::Settings, false, window, cx);
        editor.update(cx, |editor, cx| editor.focus_search(window, cx));
    }

    /// Opens `settings.json` in a file tab for good.
    pub(super) fn open_user_settings_json(
        &mut self,
        _: &crate::OpenUserSettingsJson,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.settings_file_key(cx) else {
            return;
        };
        // VS Code makes a missing file before opening it
        // (`createIfNotExists`), with an empty object that changes nothing.
        if !key.exists()
            && let Err(error) = json_file::write_text(&key, "{\n}")
        {
            let notice = Notification::error(format!("无法创建用户设置：{error}"));
            window.push_notification(notice, cx);
            return;
        }
        self.open_tab(TabKey::File(key), false, window, cx);
    }

    /// Follows the store's error: a file that stopped parsing brings up the
    /// notice, one that parses again takes it away.
    pub(super) fn settings_error_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let error = SettingsStore::global(cx).error().is_some();
        if error == self.settings_error_shown {
            return;
        }
        self.settings_error_shown = error;
        if error {
            let notice = Notification::error(UNREADABLE)
                .id::<SettingsErrorNotice>()
                .autohide(false)
                .content(self.open_button());
            window.push_notification(notice, cx);
        } else {
            window.remove_notification::<SettingsErrorNotice>(cx);
        }
    }
}
