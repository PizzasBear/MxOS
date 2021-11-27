use core::ops::Range;
use multiboot2::{MemoryArea, MemoryMapTag};
use x86_64::structures::paging::{FrameAllocator, PhysFrame, Size2MiB};
use x86_64::PhysAddr;

/// A very simple frame allocator, it can't deallocate any frames.
/// It will be used for setup of the main frame allocator.
#[derive(Debug)]
pub struct BumpAllocator<'a, const N: usize> {
    current_frame: usize,
    taken_areas: [Range<usize>; N],
    current_area: Option<&'a MemoryArea>,
    memory_area_index: usize,
    memory_map_tag: &'a MemoryMapTag,
}

impl<'a, const N: usize> BumpAllocator<'a, N> {
    /// Create a new BasicFrameAllocator. Taken areas are addresses that are taken by either the
    /// kernel or the Multiboot2 information structure.
    pub fn new(taken_areas: [Range<usize>; N], memory_map_tag: &'a MemoryMapTag) -> Self {
        Self {
            current_frame: 0x200000,
            current_area: memory_map_tag.memory_areas().next(),
            memory_area_index: 0,
            memory_map_tag,
            taken_areas,
        }
    }
}

unsafe impl<'a, const N: usize> FrameAllocator<Size2MiB> for BumpAllocator<'a, N> {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size2MiB>> {
        let current_area = self.current_area?;

        if self.current_frame < current_area.start_address() as usize {
            self.current_frame = current_area.start_address() as usize + 0x1fffff & !0x1fffff;
        }

        if (current_area.end_address() as usize) < self.current_frame + 0x200000 {
            self.memory_area_index += 1;
            self.current_area = self
                .memory_map_tag
                .memory_areas()
                .nth(self.memory_area_index);
            return self.allocate_frame();
        }
        for area in &self.taken_areas {
            if area.start < self.current_frame + 0x200000 && self.current_frame < area.end {
                self.current_frame = area.end + 0x1fffff & !0x1fffff;
                return self.allocate_frame();
            }
        }
        let frame = PhysFrame::from_start_address(PhysAddr::new(self.current_frame as _)).unwrap();

        self.current_frame += 0x200000;

        Some(frame)
    }
}
