// SPDX-License-Identifier: MPL-2.0
// Copyright (C) 2026 149segolte <me@149segolte.dev>

use anyhow::{Context, Result, bail, ensure};
use ash::{Entry, vk};
use serde::Deserialize;
use std::{ffi::CStr, io::Cursor, path::PathBuf};

const WORKGROUP_SIZE: u32 = 64;

#[derive(Deserialize)]
struct Shader {
    source_path: PathBuf,
    entry_point: String,
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let count = args
        .next()
        .map_or(Ok(1003), |arg| arg.parse::<u32>())
        .context("usage: cargo run -- [positive element count]")?;
    ensure!(
        count > 0 && args.next().is_none(),
        "usage: cargo run -- [positive element count]"
    );

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = root.join("target/shaders/manifest.json");
    let shaders: Vec<Shader> = serde_json::from_slice(
        &std::fs::read(&manifest).context("shader is missing; run `cargo shader` first")?,
    )
    .context("invalid cargo-gpu shader manifest")?;
    let shader = shaders
        .iter()
        .find(|s| s.entry_point == "main_cs")
        .context("main_cs entry point is missing from shader manifest")?;
    // cargo-gpu stores source_path relative to the shader crate, not the manifest.
    let bytes = std::fs::read(root.join("shader").join(&shader.source_path))
        .context("cannot read compiled shader; run `cargo shader` again")?;
    let words = ash::util::read_spv(&mut Cursor::new(bytes)).context("invalid SPIR-V")?;

    // SAFETY: Vulkan handles, allocation sizes, command ordering, and mapped-memory
    // lifetimes are managed by Compute. The loaded Entry outlives every handle.
    unsafe {
        let entry = Entry::linked();
        let mut compute = Compute::new(&entry)?;
        compute.run(count, &words)?;
    }
    Ok(())
}

// Store each handle immediately after creation so partial initialization also
// cleans up correctly. Null handles are valid arguments to Vulkan destroy calls.
struct Compute {
    instance: ash::Instance,
    device: Option<ash::Device>,
    physical: vk::PhysicalDevice,
    family: u32,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    descriptor_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    pipeline_layout: vk::PipelineLayout,
    shader: vk::ShaderModule,
    pipeline: vk::Pipeline,
    command_pool: vk::CommandPool,
    fence: vk::Fence,
}

impl Compute {
    unsafe fn new(entry: &Entry) -> Result<Self> {
        let extensions = unsafe { entry.enumerate_instance_extension_properties(None)? };
        let portability = extensions.iter().any(|e| unsafe {
            CStr::from_ptr(e.extension_name.as_ptr()) == ash::khr::portability_enumeration::NAME
        });
        let names = if portability {
            vec![ash::khr::portability_enumeration::NAME.as_ptr()]
        } else {
            vec![]
        };
        let app = vk::ApplicationInfo::default()
            .application_name(c"rust-gpu compute")
            .api_version(vk::API_VERSION_1_1);
        let info = vk::InstanceCreateInfo::default()
            .application_info(&app)
            .enabled_extension_names(&names)
            .flags(if portability {
                vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR
            } else {
                vk::InstanceCreateFlags::empty()
            });
        let instance = unsafe { entry.create_instance(&info, None) }
            .context("cannot create Vulkan 1.1 instance; check your Vulkan driver installation")?;
        let mut result = Self {
            instance,
            device: None,
            physical: vk::PhysicalDevice::null(),
            family: 0,
            buffer: vk::Buffer::null(),
            memory: vk::DeviceMemory::null(),
            descriptor_layout: vk::DescriptorSetLayout::null(),
            descriptor_pool: vk::DescriptorPool::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            shader: vk::ShaderModule::null(),
            pipeline: vk::Pipeline::null(),
            command_pool: vk::CommandPool::null(),
            fence: vk::Fence::null(),
        };
        for physical in unsafe { result.instance.enumerate_physical_devices()? } {
            let props = unsafe { result.instance.get_physical_device_properties(physical) };
            if props.api_version < vk::API_VERSION_1_1 {
                continue;
            }
            let families = unsafe {
                result
                    .instance
                    .get_physical_device_queue_family_properties(physical)
            };
            if let Some(family) = families
                .iter()
                .position(|f| f.queue_count > 0 && f.queue_flags.contains(vk::QueueFlags::COMPUTE))
            {
                result.physical = physical;
                result.family = family as u32;
                println!(
                    "Device: {}",
                    unsafe { CStr::from_ptr(props.device_name.as_ptr()) }.to_string_lossy()
                );
                break;
            }
        }
        ensure!(
            result.physical != vk::PhysicalDevice::null(),
            "no Vulkan 1.1 device with a compute queue found"
        );
        let extensions = unsafe {
            result
                .instance
                .enumerate_device_extension_properties(result.physical)?
        };
        let names: Vec<_> = extensions
            .iter()
            .filter(|e| unsafe {
                CStr::from_ptr(e.extension_name.as_ptr()) == ash::khr::portability_subset::NAME
            })
            .map(|_| ash::khr::portability_subset::NAME.as_ptr())
            .collect();
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(result.family)
            .queue_priorities(&priorities)];
        let info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queues)
            .enabled_extension_names(&names);
        result.device = Some(unsafe {
            result
                .instance
                .create_device(result.physical, &info, None)?
        });
        Ok(result)
    }

    unsafe fn run(&mut self, count: u32, words: &[u32]) -> Result<()> {
        let device = self.device.as_ref().context("device not initialized")?;
        let size = u64::from(count) * size_of::<u32>() as u64;
        let props = unsafe { self.instance.get_physical_device_properties(self.physical) };
        let groups = count.div_ceil(WORKGROUP_SIZE);
        ensure!(
            size <= u64::from(props.limits.max_storage_buffer_range),
            "input exceeds maxStorageBufferRange"
        );
        ensure!(
            groups <= props.limits.max_compute_work_group_count[0],
            "input exceeds maxComputeWorkGroupCount"
        );
        // Explicit unsafe block keeps raw Vulkan calls together. All dependencies
        // are destroyed in reverse order in Drop, including on errors.
        unsafe {
            self.buffer = device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )?;
            let requirements = device.get_buffer_memory_requirements(self.buffer);
            let memory = self
                .instance
                .get_physical_device_memory_properties(self.physical);
            let index = (0..memory.memory_type_count)
                .find(|i| {
                    requirements.memory_type_bits & (1 << i) != 0
                        && memory.memory_types[*i as usize].property_flags.contains(
                            vk::MemoryPropertyFlags::HOST_VISIBLE
                                | vk::MemoryPropertyFlags::HOST_COHERENT,
                        )
                })
                .context("no host-visible coherent storage-buffer memory type available")?;
            self.memory = device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(index),
                None,
            )?;
            device.bind_buffer_memory(self.buffer, self.memory, 0)?;
            let pointer = device
                .map_memory(self.memory, 0, size, vk::MemoryMapFlags::empty())?
                .cast::<u32>();
            for i in 0..count {
                pointer.add(i as usize).write(i);
            }
            device.unmap_memory(self.memory);

            let bindings = [vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)];
            self.descriptor_layout = device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )?;
            let layouts = [self.descriptor_layout];
            self.pipeline_layout = device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts),
                None,
            )?;
            self.shader = device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(words), None)?;
            let stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(self.shader)
                .name(c"main_cs");
            let pipeline_info = [vk::ComputePipelineCreateInfo::default()
                .stage(stage)
                .layout(self.pipeline_layout)];
            self.pipeline = match device.create_compute_pipelines(
                vk::PipelineCache::null(),
                &pipeline_info,
                None,
            ) {
                Ok(pipelines) => pipelines[0],
                Err((pipelines, error)) => {
                    for pipeline in pipelines {
                        device.destroy_pipeline(pipeline, None);
                    }
                    return Err(error.into());
                }
            };
            let pool_sizes = [vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)];
            self.descriptor_pool = device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(1)
                    .pool_sizes(&pool_sizes),
                None,
            )?;
            let set = device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.descriptor_pool)
                    .set_layouts(&layouts),
            )?[0];
            let buffer_info = [vk::DescriptorBufferInfo::default()
                .buffer(self.buffer)
                .offset(0)
                .range(size)];
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&buffer_info)],
                &[],
            );

            self.command_pool = device.create_command_pool(
                &vk::CommandPoolCreateInfo::default().queue_family_index(self.family),
                None,
            )?;
            let command = device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?[0];
            device.begin_command_buffer(
                command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            device.cmd_bind_pipeline(command, vk::PipelineBindPoint::COMPUTE, self.pipeline);
            device.cmd_bind_descriptor_sets(
                command,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_layout,
                0,
                &[set],
                &[],
            );
            // Queue submission makes the coherent host writes available to the GPU.
            device.cmd_dispatch(command, groups, 1, 1);
            let barrier = [vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                .dst_access_mask(vk::AccessFlags::HOST_READ)];
            device.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::PipelineStageFlags::HOST,
                vk::DependencyFlags::empty(),
                &barrier,
                &[],
                &[],
            );
            device.end_command_buffer(command)?;
            self.fence = device.create_fence(&vk::FenceCreateInfo::default(), None)?;
            let commands = [command];
            let submits = [vk::SubmitInfo::default().command_buffers(&commands)];
            device.queue_submit(
                device.get_device_queue(self.family, 0),
                &submits,
                self.fence,
            )?;
            device.wait_for_fences(&[self.fence], true, u64::MAX)?;

            let pointer = device
                .map_memory(self.memory, 0, size, vk::MemoryMapFlags::empty())?
                .cast::<u32>();
            let values = std::slice::from_raw_parts(pointer, count as usize);
            let mismatch = values.iter().enumerate().find_map(|(i, &actual)| {
                let expected = (i as u32).wrapping_mul(2).wrapping_add(1);
                (actual != expected).then_some((i, actual, expected))
            });
            let preview = values[..values.len().min(8)].to_vec();
            device.unmap_memory(self.memory);
            if let Some((i, actual, expected)) = mismatch {
                bail!("GPU verification failed at {i}: got {actual}, expected {expected}");
            }
            println!("Verified {count} values across {groups} workgroups: {preview:?}");
        }
        Ok(())
    }
}

impl Drop for Compute {
    fn drop(&mut self) {
        // SAFETY: All handles belong to this instance/device. Idle wait completes
        // submitted work before any resources used by that work are destroyed.
        unsafe {
            if let Some(device) = &self.device {
                let _ = device.device_wait_idle();
                device.destroy_fence(self.fence, None);
                device.destroy_command_pool(self.command_pool, None);
                device.destroy_descriptor_pool(self.descriptor_pool, None);
                device.destroy_pipeline(self.pipeline, None);
                device.destroy_shader_module(self.shader, None);
                device.destroy_pipeline_layout(self.pipeline_layout, None);
                device.destroy_descriptor_set_layout(self.descriptor_layout, None);
                device.destroy_buffer(self.buffer, None);
                device.free_memory(self.memory, None);
                device.destroy_device(None);
            }
            self.instance.destroy_instance(None);
        }
    }
}
