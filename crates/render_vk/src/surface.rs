use ash::{ext, khr, vk, Entry, Instance};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use winit::window::Window;

use crate::RenderError;

pub fn create_surface(
    entry: &Entry,
    instance: &Instance,
    window: &Window,
) -> Result<vk::SurfaceKHR, RenderError> {
    let display = window
        .display_handle()
        .map_err(|e| RenderError::Initialization(format!("display handle error: {e}")))?;
    let handle = window
        .window_handle()
        .map_err(|e| RenderError::Initialization(format!("window handle error: {e}")))?;
    unsafe { platform::create_surface(entry, instance, display.as_raw(), handle.as_raw()) }
}

pub fn required_extensions(
    window: &Window,
    enable_validation: bool,
) -> Result<Vec<*const i8>, RenderError> {
    let display = window
        .display_handle()
        .map_err(|e| RenderError::Initialization(format!("display handle error: {e}")))?;
    platform::required_extensions(display.as_raw(), enable_validation)
}

#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
mod platform {
    use super::*;

    pub(super) unsafe fn create_surface(
        entry: &Entry,
        instance: &Instance,
        display: RawDisplayHandle,
        handle: RawWindowHandle,
    ) -> Result<vk::SurfaceKHR, RenderError> {
        match (display, handle) {
            (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(window)) => {
                let create_info = vk::WaylandSurfaceCreateInfoKHR::default()
                    .display(display.display.as_ptr() as *mut _)
                    .surface(window.surface.as_ptr() as *mut _);
                let wayland = khr::wayland_surface::Instance::new(entry, instance);
                Ok(wayland.create_wayland_surface(&create_info, None)?)
            }
            (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(window)) => {
                let dpy = display
                    .display
                    .map_or(std::ptr::null_mut(), |ptr| ptr.as_ptr());
                let create_info = vk::XlibSurfaceCreateInfoKHR::default()
                    .dpy(dpy as *mut _)
                    .window(window.window);
                let xlib = khr::xlib_surface::Instance::new(entry, instance);
                Ok(xlib.create_xlib_surface(&create_info, None)?)
            }
            (RawDisplayHandle::Xcb(display), RawWindowHandle::Xcb(window)) => {
                let connection = display
                    .connection
                    .map_or(std::ptr::null_mut(), |ptr| ptr.as_ptr());
                let create_info = vk::XcbSurfaceCreateInfoKHR::default()
                    .connection(connection as *mut _)
                    .window(window.window.get());
                let xcb = khr::xcb_surface::Instance::new(entry, instance);
                Ok(xcb.create_xcb_surface(&create_info, None)?)
            }
            _ => Err(RenderError::UnsupportedPlatform(
                "Windowing platform is not supported on this build".into(),
            )),
        }
    }

    pub(super) fn required_extensions(
        display: RawDisplayHandle,
        enable_validation: bool,
    ) -> Result<Vec<*const i8>, RenderError> {
        let mut extensions = vec![khr::surface::NAME.as_ptr()];
        match display {
            RawDisplayHandle::Wayland(_) => {
                extensions.push(khr::wayland_surface::NAME.as_ptr());
            }
            RawDisplayHandle::Xlib(_) => {
                extensions.push(khr::xlib_surface::NAME.as_ptr());
            }
            RawDisplayHandle::Xcb(_) => {
                extensions.push(khr::xcb_surface::NAME.as_ptr());
            }
            _ => {
                return Err(RenderError::UnsupportedPlatform(
                    "Windowing platform is not supported on this build".into(),
                ))
            }
        }
        if enable_validation {
            extensions.push(ext::debug_utils::NAME.as_ptr());
        }
        Ok(extensions)
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
)))]
mod platform {
    use super::*;

    pub(super) unsafe fn create_surface(
        _entry: &Entry,
        _instance: &Instance,
        _display: RawDisplayHandle,
        _handle: RawWindowHandle,
    ) -> Result<vk::SurfaceKHR, RenderError> {
        Err(RenderError::UnsupportedPlatform(
            "Surface creation helper not implemented for this OS".into(),
        ))
    }

    pub(super) fn required_extensions(
        _display: RawDisplayHandle,
        _enable_validation: bool,
    ) -> Result<Vec<*const i8>, RenderError> {
        Err(RenderError::UnsupportedPlatform(
            "Surface helper not implemented for this OS".into(),
        ))
    }
}
