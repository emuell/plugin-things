use cursor_icon::CursorIcon;
use raw_window_handle::{RawWindowHandle, HasWindowHandle, HasDisplayHandle};

use crate::platform::os_window_handle::OsWindowHandle;
use crate::LogicalPosition;
use crate::dimensions::LogicalSize;
use crate::error::Error;
use crate::event::EventCallback;
use crate::platform::{window::OsWindow, interface::OsWindowInterface};
use crate::screen::screen_scale;

#[derive(Clone)]
pub struct WindowAttributes {
    pub(crate) size: LogicalSize,
    pub(crate) scale: f64,
    pub(crate) software_rendering: bool,
}

impl WindowAttributes {
    pub fn new(size: LogicalSize, scale: f64) -> Self {
        let software_rendering = false;
        Self {
            size,
            scale,
            software_rendering,
        }
    }

    pub fn with_size(size: LogicalSize) -> Self {
        Self::new(size, 1.0)
    }

    /// Force using software renderers for drawing instead of the platform's default GPU renderer.
    pub fn with_software_rendering(mut self, software_rendering: bool) -> Self {
        self.software_rendering = software_rendering;
        self
    }

    pub fn software_rendering(&self) -> bool {
        self.software_rendering
    }

    pub fn size(&self) -> LogicalSize {
        self.size
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    pub fn scaled_size(&self) -> LogicalSize {
        self.size * self.scale
    }
}

pub struct Window {
    attributes: WindowAttributes,
    os_window_handle: OsWindowHandle,
}

impl Window {
    // Returns a reasonable default scale for the window based on the platform
    pub fn default_scale() -> f64 {
        if cfg!(target_os = "macos") {
            // Mac window sizes are in logical coordinates so no need to scale
            1.0
        } else {
            screen_scale()
        }
    }

    pub fn open(
        parent: RawWindowHandle,
        attributes: WindowAttributes,
        event_callback: Box<EventCallback>,
    ) -> Result<Window, Error> {
        let os_window_handle = OsWindow::open(
            parent,
            attributes.clone(),
            event_callback,
        )?;

        Ok(Self {
            attributes,
            os_window_handle,
        })
    }

    pub fn attributes(&self) -> &WindowAttributes {
        &self.attributes
    }

    pub fn resized(&self, size: LogicalSize, scale: f64) {
        self.os_window_handle.resized(size, scale);
    }

    /// This only needs to be called on Linux
    pub fn poll_events(&self) -> Result<(), Error> {
        self.os_window_handle.poll_events()
    }

    pub fn set_cursor(&self, cursor: Option<CursorIcon>) {
        self.os_window_handle.set_cursor(cursor);
    }

    pub fn set_input_focus(&self, focus: bool) {
        self.os_window_handle.set_input_focus(focus);
    }

    pub fn warp_mouse(&self, position: LogicalPosition) {
        self.os_window_handle.warp_mouse(position);
    }
}

impl HasWindowHandle for Window {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        self.os_window_handle.window_handle()
    }
}

impl HasDisplayHandle for Window {
    fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        self.os_window_handle.display_handle()
    }
}
