//! Windows UI Automation (UIA) input provider.
//!
//! WarpUI paints its own text input and hosts no native OS edit control, so on
//! Windows there is no UIA element for third-party dictation/automation tools
//! (Superwhisper, Windows Narrator, `Inspect.exe`, etc.) to target when they
//! insert transcribed text automatically. This module registers an
//! [`accesskit_windows::SubclassingAdapter`] against the window's `HWND` so
//! those tools find a focused text element that supports programmatic insertion.
//!
//! The portable tree/action logic lives in
//! [`warpui_core::accessibility`] (and is unit-tested there); this module is the
//! Windows-specific glue: it owns the COM adapter, forwards focus/content
//! changes into the accesskit tree, and translates the resulting UIA
//! set-value/insert actions into a [`CustomEvent::AccessibilityInsertText`] that
//! the event loop feeds into WarpUI's normal typed-text path.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, TreeUpdate};
use accesskit_windows::{HWND, SubclassingAdapter};
use winit::event_loop::EventLoopProxy;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window as WinitWindow;

use crate::accessibility::{
    AccessibilityContent, build_focused_input_tree, text_to_insert_for_action,
};
use crate::windowing::winit::app::CustomEvent;

thread_local! {
    /// Holds a provider created during window construction until [`open_window`]
    /// takes ownership of it. The adapter must be created while the window is
    /// still hidden (accesskit's subclassing adapter panics on a visible
    /// window), which happens inside `create_window` before it stores the
    /// window's `Inner`; this slot carries it the short distance to `Inner`.
    ///
    /// [`open_window`]: super::super::window::Window::open_window
    static PENDING_PROVIDER: RefCell<Option<WindowsAccessibility>> = const { RefCell::new(None) };
}

/// Latest focused-view content, shared between the adapter's handlers and the
/// window that pushes updates. The activation handler reads it to build the
/// initial tree; the action handler reads it to decide whether an editable
/// input is focused. Guarded by a mutex because the action handler may run on a
/// thread other than the one that owns the window.
#[derive(Default)]
struct SharedState {
    latest: Mutex<Option<AccessibilityContent>>,
}

impl SharedState {
    fn has_focused_editable(&self) -> bool {
        self.latest
            .lock()
            .ok()
            .and_then(|content| content.as_ref().map(|c| c.role.is_editable_text()))
            .unwrap_or(false)
    }

    fn tree(&self) -> Option<TreeUpdate> {
        self.latest
            .lock()
            .ok()
            .and_then(|content| content.as_ref().map(build_focused_input_tree))
    }
}

/// Provides the initial accesskit tree when a UIA client first attaches.
struct WarpActivationHandler {
    shared: Arc<SharedState>,
}

impl ActivationHandler for WarpActivationHandler {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.shared.tree()
    }
}

/// Handles UIA actions (value set / text insertion) from a client, routing the
/// resulting text into WarpUI's typed-text path via the event loop.
struct WarpActionHandler {
    shared: Arc<SharedState>,
    proxy: EventLoopProxy<CustomEvent>,
}

impl ActionHandler for WarpActionHandler {
    fn do_action(&mut self, request: ActionRequest) {
        if let Some(text) = text_to_insert_for_action(
            request.action,
            request.data.as_ref(),
            self.shared.has_focused_editable(),
        ) {
            let _ = self
                .proxy
                .send_event(CustomEvent::AccessibilityInsertText(text));
        }
    }
}

/// Owns the UIA adapter for one window. Dropping it uninstalls the window
/// subclass, so its lifetime is tied to the window's `Inner`.
pub struct WindowsAccessibility {
    adapter: RefCell<SubclassingAdapter>,
    shared: Arc<SharedState>,
}

impl WindowsAccessibility {
    /// Creates the UIA provider for `window`.
    ///
    /// Must be called while the window is still hidden: accesskit's subclassing
    /// adapter panics if the window is already visible. Returns `None` if the
    /// window does not expose a Win32 handle.
    fn new(window: &WinitWindow, proxy: EventLoopProxy<CustomEvent>) -> Option<Self> {
        let hwnd = hwnd_for(window)?;
        let shared = Arc::new(SharedState::default());
        let adapter = SubclassingAdapter::new(
            hwnd,
            WarpActivationHandler {
                shared: shared.clone(),
            },
            WarpActionHandler {
                shared: shared.clone(),
                proxy,
            },
        );
        Some(Self {
            adapter: RefCell::new(adapter),
            shared,
        })
    }

    /// Updates the exposed tree to reflect the newly focused view so a UIA
    /// client sees the correct focused text element and value.
    pub fn update(&self, content: AccessibilityContent) {
        let update = build_focused_input_tree(&content);
        if let Ok(mut latest) = self.shared.latest.lock() {
            *latest = Some(content);
        }
        if let Some(events) = self.adapter.borrow_mut().update_if_active(|| update) {
            events.raise();
        }
    }
}

/// Creates a UIA provider for `window` and stashes it until [`take_pending`]
/// moves it into the window's `Inner`. Called from `create_window` while the
/// window is still hidden.
pub fn stash_pending(window: &WinitWindow, proxy: EventLoopProxy<CustomEvent>) {
    if let Some(provider) = WindowsAccessibility::new(window, proxy) {
        PENDING_PROVIDER.with(|slot| *slot.borrow_mut() = Some(provider));
    }
}

/// Takes ownership of the provider stashed by the most recent [`stash_pending`].
pub fn take_pending() -> Option<WindowsAccessibility> {
    PENDING_PROVIDER.with(|slot| slot.borrow_mut().take())
}

fn hwnd_for(window: &WinitWindow) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut std::ffi::c_void)),
        _ => None,
    }
}
