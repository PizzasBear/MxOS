use core::ops::Range;
use multiboot2::{BootInformation, MemoryArea, MemoryMapTag};
use x86_64::structures::paging::{FrameAllocator, PageTable, PageTableFlags, PhysFrame, Size4KiB};
use x86_64::{PhysAddr, VirtAddr};

/// A very simple frame allocator, it can't deallocate any frames.
/// It will be used for setup of the main frame allocator.
#[derive(Debug)]
pub struct BasicFrameAllocator<'a> {
    current_frame: u64,
    taken_areas: [Range<u64>; 2],
    current_area: Option<&'a MemoryArea>,
    memory_area_index: usize,
    memory_map_tag: &'a MemoryMapTag,
}

impl<'a> BasicFrameAllocator<'a> {
    /// Create a new BasicFrameAllocator. Taken areas .
    pub fn new(taken_areas: [Range<u64>; 2], memory_map_tag: &'a MemoryMapTag) -> Self {
        Self {
            current_frame: 4096,
            current_area: memory_map_tag.memory_areas().next(),
            memory_area_index: 0,
            memory_map_tag,
            taken_areas,
        }
    }
}

unsafe impl<'a> FrameAllocator<Size4KiB> for BasicFrameAllocator<'a> {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        let current_area = self.current_area?;

        if self.current_frame < current_area.start_address() {
            self.current_frame = current_area.start_address() + 4095 & !4095;
        }

        if current_area.end_address() < self.current_frame + 4096 {
            self.memory_area_index += 1;
            self.current_area = self
                .memory_map_tag
                .memory_areas()
                .nth(self.memory_area_index);
            return self.allocate_frame();
        }
        for area in &self.taken_areas {
            if area.start < self.current_frame + 4096 && self.current_frame < area.end {
                self.current_frame = area.end + 4095 & !4095;
                return self.allocate_frame();
            }
        }
        self.current_frame += 4096;

        Some(PhysFrame::from_start_address(PhysAddr::new(self.current_frame - 4096)).unwrap())
    }
}

pub unsafe fn reset_page_table<FA: FrameAllocator<Size4KiB>>(
    kernel_start: u64,
    kernel_end: u64,
    boot_info: &BootInformation,
    frame_allocator: &mut FA,
) {
    // use core::ptr;
    use x86_64::registers::control::Cr3;

    let mut frames = [PhysFrame::from_start_address(PhysAddr::new(0)).unwrap(); 16];
    frames[0] = frame_allocator.allocate_frame().unwrap();
    let mut frames_start = 0;
    let mut frames_len = 1;
    // let virt_addr = VirtAddr::new(frames[0].start_address().as_u64());

    let new_level_4_page_frame = frames[0];
    let new_level_4_page = &mut *(frames[0].start_address().as_u64() as *mut PageTable);
    new_level_4_page.zero();

    for addr in ((kernel_start & !4095)..(kernel_end + 4095 & !4095))
        .step_by(4096)
        .chain(
            ((boot_info.start_address() as u64 & !4095)
                ..(boot_info.end_address() as u64 + 4095 & !4095))
                .step_by(4096),
        )
    {
        let virt_addr = VirtAddr::new(addr);

        let p4_entry = &mut new_level_4_page[virt_addr.p4_index()];
        if p4_entry.is_unused() {
            let frame = frame_allocator.allocate_frame().unwrap();
            frames[(frames_start + frames_len) % frames.len()] = frame;
            frames_len += 1;
            assert!(frames_len <= frames.len(), "Too many frames");

            p4_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
            let p3 = &mut *(p4_entry.addr().as_u64() as *mut PageTable);
            p3.zero();
        }
        let p3 = &mut *(p4_entry.addr().as_u64() as *mut PageTable);

        let p3_entry = &mut p3[virt_addr.p3_index()];
        if p3_entry.is_unused() {
            let frame = frame_allocator.allocate_frame().unwrap();
            frames[(frames_start + frames_len) % frames.len()] = frame;
            frames_len += 1;
            assert!(frames_len <= frames.len(), "Too many frames");

            p3_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
            let p2 = &mut *(p3_entry.addr().as_u64() as *mut PageTable);
            p2.zero();
        }
        let p2 = &mut *(p3_entry.addr().as_u64() as *mut PageTable);

        let p2_entry = &mut p2[virt_addr.p2_index()];
        if p2_entry.is_unused() {
            let frame = frame_allocator.allocate_frame().unwrap();
            frames[(frames_start + frames_len) % frames.len()] = frame;
            frames_len += 1;
            assert!(frames_len <= frames.len(), "Too many frames");

            p2_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
            let p1 = &mut *(p2_entry.addr().as_u64() as *mut PageTable);
            p1.zero();
        }
        let p1 = &mut *(p2_entry.addr().as_u64() as *mut PageTable);

        let p1_entry = &mut p1[virt_addr.p1_index()];
        assert!(p1_entry.is_unused());
        p1_entry.set_frame(
            PhysFrame::from_start_address(PhysAddr::new(addr)).unwrap(),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
        );

        // Allocate frames for the pages
        while 0 < frames_len {
            let addr = frames[frames_start].start_address().as_u64();
            let virt_addr = VirtAddr::new(addr);

            frames_start = (frames_start + 1) % frames.len();
            frames_len -= 1;

            let p4_entry = &mut new_level_4_page[virt_addr.p4_index()];
            if p4_entry.is_unused() {
                let frame = frame_allocator.allocate_frame().unwrap();
                frames[(frames_start + frames_len) % frames.len()] = frame;
                frames_len += 1;
                assert!(frames_len <= frames.len(), "Too many frames");

                p4_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
                let p3 = &mut *(p4_entry.addr().as_u64() as *mut PageTable);
                p3.zero();
            }
            let p3 = &mut *(p4_entry.addr().as_u64() as *mut PageTable);

            let p3_entry = &mut p3[virt_addr.p3_index()];
            if p3_entry.is_unused() {
                let frame = frame_allocator.allocate_frame().unwrap();
                frames[(frames_start + frames_len) % frames.len()] = frame;
                frames_len += 1;
                assert!(frames_len <= frames.len(), "Too many frames");

                p3_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
                let p2 = &mut *(p3_entry.addr().as_u64() as *mut PageTable);
                p2.zero();
            }
            let p2 = &mut *(p3_entry.addr().as_u64() as *mut PageTable);

            let p2_entry = &mut p2[virt_addr.p2_index()];
            if p2_entry.is_unused() {
                let frame = frame_allocator.allocate_frame().unwrap();
                frames[(frames_start + frames_len) % frames.len()] = frame;
                frames_len += 1;
                assert!(frames_len <= frames.len(), "Too many frames");

                p2_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
                let p1 = &mut *(p2_entry.addr().as_u64() as *mut PageTable);
                p1.zero();
            }
            let p1 = &mut *(p2_entry.addr().as_u64() as *mut PageTable);

            let p1_entry = &mut p1[virt_addr.p1_index()];
            assert!(p1_entry.is_unused());
            p1_entry.set_frame(
                PhysFrame::from_start_address(PhysAddr::new(addr)).unwrap(),
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
            );
        }
    }

    log::info!("Resetting the page table...");
    let (_, cr3_flags) = Cr3::read();
    Cr3::write(new_level_4_page_frame, cr3_flags);
    log::info!("The page table is reset");
}
