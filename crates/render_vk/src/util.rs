use ash::vk;

use crate::RenderError;

pub(crate) fn create_image(
    device: &ash::Device,
    width: u32,
    height: u32,
    format: vk::Format,
    tiling: vk::ImageTiling,
    usage: vk::ImageUsageFlags,
    properties: vk::MemoryPropertyFlags,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    samples: vk::SampleCountFlags,
) -> Result<(vk::Image, vk::DeviceMemory), RenderError> {
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .format(format)
        .tiling(tiling)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .samples(samples);

    let image = unsafe { device.create_image(&image_info, None) }.map_err(RenderError::from)?;
    let mem_requirements = unsafe { device.get_image_memory_requirements(image) };

    let memory_type = find_memory_type(
        mem_requirements.memory_type_bits,
        properties,
        memory_properties,
    )
    .ok_or_else(|| RenderError::Initialization("Failed to find suitable memory type".into()))?;

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(mem_requirements.size)
        .memory_type_index(memory_type);

    let memory = unsafe { device.allocate_memory(&alloc_info, None) }.map_err(RenderError::from)?;
    unsafe {
        device
            .bind_image_memory(image, memory, 0)
            .map_err(RenderError::from)?;
    }

    Ok((image, memory))
}

pub(crate) fn create_image_view(
    device: &ash::Device,
    image: vk::Image,
    format: vk::Format,
    aspect_flags: vk::ImageAspectFlags,
) -> Result<vk::ImageView, RenderError> {
    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: aspect_flags,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    unsafe { device.create_image_view(&view_info, None) }.map_err(RenderError::from)
}

pub(crate) fn create_buffer(
    device: &ash::Device,
    size: vk::DeviceSize,
    usage: vk::BufferUsageFlags,
    properties: vk::MemoryPropertyFlags,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
) -> Result<(vk::Buffer, vk::DeviceMemory), RenderError> {
    let buffer_info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let buffer = unsafe { device.create_buffer(&buffer_info, None) }.map_err(RenderError::from)?;
    let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
    let memory_type =
        find_memory_type(requirements.memory_type_bits, properties, memory_properties).ok_or_else(
            || RenderError::Initialization("Failed to find suitable memory type".into()),
        )?;
    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type);
    let memory = unsafe { device.allocate_memory(&alloc_info, None) }.map_err(RenderError::from)?;
    unsafe {
        device.bind_buffer_memory(buffer, memory, 0)?;
    }
    Ok((buffer, memory))
}

pub(crate) fn find_memory_type(
    type_filter: u32,
    properties: vk::MemoryPropertyFlags,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
) -> Option<u32> {
    for i in 0..memory_properties.memory_type_count {
        let suitable = (type_filter & (1 << i)) != 0;
        let supported = memory_properties.memory_types[i as usize]
            .property_flags
            .contains(properties);
        if suitable && supported {
            return Some(i);
        }
    }
    None
}

