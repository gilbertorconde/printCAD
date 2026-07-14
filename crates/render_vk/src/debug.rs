//! Vulkan validation-layer output routed into `tracing`.
//!
//! Without a messenger the validation layers only print through the loader's
//! default stderr handler, invisible to the app's log pipeline.

use std::ffi::{c_void, CStr};

use ash::{ext::debug_utils, vk, Entry};
use tracing::{debug, error, trace, warn};

pub(crate) struct DebugMessenger {
    loader: debug_utils::Instance,
    messenger: vk::DebugUtilsMessengerEXT,
}

impl DebugMessenger {
    /// Create-info shared by the messenger itself and the instance
    /// `push_next` chain (the latter captures create/destroy-time messages).
    pub(crate) fn create_info<'a>() -> vk::DebugUtilsMessengerCreateInfoEXT<'a> {
        vk::DebugUtilsMessengerCreateInfoEXT::default()
            .message_severity(
                vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                    | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                    | vk::DebugUtilsMessageSeverityFlagsEXT::INFO,
            )
            .message_type(
                vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                    | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                    | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
            )
            .pfn_user_callback(Some(vulkan_debug_callback))
    }

    pub(crate) fn new(entry: &Entry, instance: &ash::Instance) -> Result<Self, vk::Result> {
        let loader = debug_utils::Instance::new(entry, instance);
        let messenger = unsafe { loader.create_debug_utils_messenger(&Self::create_info(), None)? };
        Ok(Self { loader, messenger })
    }

    /// Must be called before the instance is destroyed.
    pub(crate) fn destroy(&mut self) {
        unsafe {
            self.loader
                .destroy_debug_utils_messenger(self.messenger, None)
        };
    }
}

unsafe extern "system" fn vulkan_debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user_data: *mut c_void,
) -> vk::Bool32 {
    if p_callback_data.is_null() {
        return vk::FALSE;
    }
    let data = &*p_callback_data;
    let message = if data.p_message.is_null() {
        String::new()
    } else {
        CStr::from_ptr(data.p_message)
            .to_string_lossy()
            .into_owned()
    };
    let id_name = if data.p_message_id_name.is_null() {
        String::new()
    } else {
        CStr::from_ptr(data.p_message_id_name)
            .to_string_lossy()
            .into_owned()
    };

    let kind = if message_type.contains(vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION) {
        "validation"
    } else if message_type.contains(vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE) {
        "performance"
    } else {
        "general"
    };

    match severity {
        s if s.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) => {
            error!(target: "printcad.vulkan", kind, id = %id_name, "{message}");
        }
        s if s.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) => {
            warn!(target: "printcad.vulkan", kind, id = %id_name, "{message}");
        }
        s if s.contains(vk::DebugUtilsMessageSeverityFlagsEXT::INFO) => {
            debug!(target: "printcad.vulkan", kind, id = %id_name, "{message}");
        }
        _ => {
            trace!(target: "printcad.vulkan", kind, id = %id_name, "{message}");
        }
    }

    // Per spec, the callback must always return VK_FALSE.
    vk::FALSE
}
