// SPDX-License-Identifier: MPL-2.0
// Copyright (C) 2026 149segolte <me@149segolte.dev>

#![no_std]

use spirv_std::{glam::UVec3, spirv};

// Keep the workgroup size in sync with WORKGROUP_SIZE in src/main.rs.
#[spirv(compute(threads(64)))]
pub fn main_cs(
    #[spirv(global_invocation_id)] id: UVec3,
    #[spirv(storage_buffer, descriptor_set = 0, binding = 0)] values: &mut [u32],
) {
    let index = id.x as usize;
    // The host rounds up the dispatch, so the final workgroup may be partial.
    if index < values.len() {
        values[index] = values[index].wrapping_mul(2).wrapping_add(1);
    }
}
