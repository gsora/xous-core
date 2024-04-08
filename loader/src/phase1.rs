use core::{mem, slice};

#[cfg(feature = "atsama5d27")]
pub use crate::platform::atsama5d27::load::InitialProcess;
use crate::*;

#[repr(C)]
#[cfg(not(feature = "atsama5d27"))]
pub struct InitialProcess {
    /// The RISC-V SATP value, which includes the offset of the root page
    /// table plus the process ID.
    pub satp: usize,

    /// Where execution begins
    pub entrypoint: usize,

    /// Address of the top of the stack
    pub sp: usize,

    /// Address of the start of the env block
    pub env: usize,

    #[cfg(feature = "swap")]
    pub swap_root: usize,
}

/// Phase 1:
///
/// Copy processes from FLASH to RAM, allocating memory one page at a time starting from high
/// addresses and working down. The allocations are computed from the kernel arguments, and the
/// allocated amount is re-computed and used in phase 2 to setup the page tables.
///
/// We don't memorize the allocated results (in part because we don't have malloc/alloc to stick
/// the table, and we don't know a priori how big it will be); we simply memorize the maximum extent,
/// after which we allocate the book-keeping tables.
pub fn phase_1(cfg: &mut BootConfig) {
    // Allocate space for the stack pointer.
    // The bootloader should have placed the stack pointer at the end of RAM
    // prior to jumping to our program, so allocate one page of data for
    // stack.
    // All other allocations will be placed below the stack pointer.
    //
    // As of Xous 0.8, the top page is bootloader stack, and the page below that is the 'clean suspend' page.
    cfg.init_size += GUARD_MEMORY_BYTES;

    // The first region is defined as being "main RAM", which will be used
    // to keep track of allocations.
    println!("Allocating regions");
    allocate_regions(cfg);

    // The kernel, as well as initial processes, are all stored in RAM.
    println!("Allocating processes");
    allocate_processes(cfg);

    // Copy the arguments, if requested
    if cfg.no_copy {
        // TODO: place args into cfg.args
    } else {
        println!("Copying args");
        copy_args(cfg);
    }

    // All further allocations must be page-aligned.
    cfg.init_size = (cfg.init_size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

    #[cfg(feature = "swap")]
    allocate_swap(cfg);

    // Additionally, from this point on all allocations come from
    // their respective processes rather than kernel memory.

    // Copy the processes to RAM, if requested.
    if !cfg.no_copy {
        println!("Copying processes");
        copy_processes(cfg);
    }

    // Mark all pages as in-use by the kernel.
    // NOTE: This causes the .text section to be owned by the kernel!  This
    // will require us to transfer ownership in `stage3`.
    // Note also that we skip the first four indices, causing the stack to be
    // returned to the process pool.

    // We also skip the an additional index as that is the clean suspend page. This
    // needs to be claimed by the susres server before the kernel allocates it.
    // Lower numbered indices corresponding to higher address pages.
    println!("Marking pages as in-use");
    for i in 4..(cfg.init_size / PAGE_SIZE) {
        cfg.runtime_page_tracker[cfg.sram_size / PAGE_SIZE - i] = 1;
    }
}

/// Allocate and initialize memory regions.
/// Returns a pointer to the start of the memory region.
pub fn allocate_regions(cfg: &mut BootConfig) {
    // Number of individual pages in the system
    let mut rpt_pages = cfg.sram_size / PAGE_SIZE;

    for region in cfg.regions.iter() {
        println!(
            "Discovered memory region {:08x} ({:08x} - {:08x}) -- {} bytes",
            region.name,
            region.start,
            region.start + region.length,
            region.length
        );
        let region_length_rounded = (region.length as usize + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        rpt_pages += region_length_rounded / PAGE_SIZE;
    }

    // Round the tracker to a multiple of the pointer size, so as to keep memory
    // operations fast.
    rpt_pages = (rpt_pages + mem::size_of::<usize>() - 1) & !(mem::size_of::<usize>() - 1);

    cfg.init_size += rpt_pages * mem::size_of::<XousPid>();

    // Clear all memory pages such that they're not owned by anyone
    let runtime_page_tracker = cfg.get_top();
    assert!((runtime_page_tracker as usize) < (cfg.sram_start as usize) + cfg.sram_size);
    unsafe {
        bzero(runtime_page_tracker, runtime_page_tracker.add(rpt_pages / mem::size_of::<usize>()));
    }

    cfg.runtime_page_tracker =
        unsafe { slice::from_raw_parts_mut(runtime_page_tracker as *mut XousPid, rpt_pages) };
}

pub fn allocate_processes(cfg: &mut BootConfig) {
    let process_count = cfg.init_process_count + 1;
    println!("Allocating tables for {} processes", process_count);
    let table_size = process_count * mem::size_of::<InitialProcess>();
    // Allocate the process table
    cfg.init_size += table_size;
    let processes = cfg.get_top();
    unsafe {
        bzero(processes, processes.add((table_size / mem::size_of::<usize>()) as usize));
    }
    cfg.processes =
        unsafe { slice::from_raw_parts_mut(processes as *mut InitialProcess, process_count as usize) };
}

#[cfg(feature = "swap")]
pub fn allocate_swap(cfg: &mut BootConfig) {
    let process_count = cfg.init_process_count + 1;
    let swap_pt_size = process_count * mem::size_of::<PageTable>();
    cfg.init_size += swap_pt_size;
    let swap_pt_base = cfg.get_top();
    unsafe { bzero(swap_pt_base, swap_pt_base.add(swap_pt_size / mem::size_of::<usize>())) }

    for (index, proc) in cfg.processes.iter_mut().enumerate() {
        proc.swap_root = swap_pt_base as usize + index * mem::size_of::<PageTable>();
    }

    // allocate a root PT entries - moved earlier in boot process because Swap needs it in Phase 1
    /*
    for pid in 2..process_count + 1 {
        #[cfg(not(feature = "atsama5d27"))]
        {
            let tt_address = cfg.alloc() as usize;
            let tt = unsafe { &mut *(tt_address as *mut PageTable) };
            cfg.map_page(tt, tt_address, PAGE_TABLE_ROOT_OFFSET, FLG_R | FLG_W | FLG_VALID, pid as XousPid);
            cfg.processes[pid - 1].satp = 0x8000_0000 | ((pid as usize) << 22) | (tt_address >> 12);
        }
        #[cfg(feature = "atsama5d27")]
        {
            let pid_idx = pid as usize - 1;

            // Allocate a page to handle the top-level memory translation
            let tt_address = cfg.alloc_l1_page_table(pid) as usize;
            cfg.processes[pid_idx].ttbr0 = tt_address;

            let translation_table = tt_address as *mut TranslationTableMemory;
            // Map all four pages of the translation table to the process' virtual address space
            for offset in 0..4 {
                let offset = offset * PAGE_SIZE;
                cfg.map_page(
                    translation_table,
                    tt_address + offset,
                    PAGE_TABLE_ROOT_OFFSET + offset,
                    FLG_R | FLG_W | FLG_VALID,
                    pid as XousPid,
                );
            }
            cfg.processes[pid - 1].asid = pid;
        };
    }*/
}

#[cfg(feature = "swap")]
pub fn copy_args(cfg: &mut BootConfig) {
    // With swap enabled, copy_args also merges the IniS arguments from the swap region into the kernel
    // arguments, and patches the length field accordingly.

    // Read in the swap arguments: should be located at beginning of the first page of swap.
    // Safety: only safe because we know that the decrypt was setup by read_swap_config(), and no pages
    // were decrypted between then and now!
    let page0 = unsafe { cfg.swap_hal.as_mut().unwrap().get_decrypt() };
    let swap_args = KernelArguments::new(page0.as_ptr() as *const usize);
    let mut j = swap_args.iter();
    // skip the first argument
    let swap_xarg = j.next().expect("couldn't read initial swap tag");

    // Merge the args list to target RAM
    // Reserve space for the primary arg list + swap args - swap's XArg structure (7 words long)
    println!("length of swap arg: {}", swap_xarg.data[0] as usize);
    let final_len = cfg.args.size() + swap_xarg.data[0] as usize - 7;
    cfg.init_size += final_len;
    let runtime_arg_buffer = cfg.get_top();
    // places the boot image kernel arguments
    unsafe {
        #[allow(clippy::cast_ptr_alignment)]
        memcpy(runtime_arg_buffer, cfg.args.base as *const usize, cfg.args.size() as usize)
    };
    // safety: this should be aligned, allowing this conversion. Note that we do violate a safety condition
    // in that we haven't fully initialized the region (the swap arg extension area is uninit), but I think
    // it's OK because we will write only to that region (but I suppose in practice, Rust could assume the
    // untouched data is 0 or something and try an optimization based on that).
    let merged_arg_slice =
        unsafe { core::slice::from_raw_parts_mut(runtime_arg_buffer as *mut usize, final_len) };
    let mut arg_index = cfg.args.size();
    println!("orig arg size: {}, new size: {}", cfg.args.size(), final_len);

    // append the swap arguments, and patch the size field accordingly
    for a in j {
        // turn the argument into a raw slice
        // this is safe because:
        //  - arguments are always guaranteed to be aligned to a word boundary by the image creator
        //  - arguments are fully initialized with no UB fields under this transformation
        // +2 is for the tag field
        if a.name == u32::from_le_bytes(*b"IniS") {
            let arg_slice = unsafe {
                core::slice::from_raw_parts(
                    a.data.as_ptr().sub(2) as *const usize, // backup by 2 to accommodate the tag field
                    a.size as usize / core::mem::size_of::<usize>() + 2,
                )
            };
            println!(
                "copying arg: 0x{:x}, size: {}. index: {}, len: {}, data: {:x?}",
                a.name,
                a.size,
                arg_index,
                arg_slice.len(),
                arg_slice
            );
            merged_arg_slice[arg_index..arg_index + arg_slice.len()].copy_from_slice(arg_slice);
            arg_index += arg_slice.len();
        } else {
            println!("Unhandled arg type: {:x}", a.name);
        }
    }

    // redirect the arg buffer to point at the newly copied arguments
    cfg.args = KernelArguments::new(runtime_arg_buffer);

    // extract the new XArg field, pointing into RAM
    let args = cfg.args;
    let mut i = args.iter();
    let xarg = i.next().expect("couldn't read initial tag");
    // patch the total length of the arguments - just jam the value into the data field of the XArg by
    // dead-reckoning to the offset
    assert!(merged_arg_slice[2] == xarg.data[0] as usize); // sanity checks the dead-reckoning
    merged_arg_slice[2] = arg_index;
    use crc::{crc16, Hasher16};
    // compute the new CRC
    let mut digest = crc16::Digest::new(crc16::X25);
    // safe because we know the entire region can map into a u8 slice with no UB
    let xarg_data = unsafe {
        core::slice::from_raw_parts(
            xarg.data.as_ptr() as *const u8,
            xarg.data.len() * core::mem::size_of::<u32>(),
        )
    };
    digest.write(&xarg_data);
    // patch the CRC
    let merged_arg_slice_u8 = unsafe {
        core::slice::from_raw_parts_mut(
            runtime_arg_buffer as *mut u8,
            final_len * core::mem::size_of::<u32>(),
        )
    };
    merged_arg_slice_u8[4..6].copy_from_slice(&digest.sum16().to_le_bytes());
}

#[cfg(feature = "swap")]
fn remaining_in_page(addr: usize) -> usize { PAGE_SIZE - (addr & (PAGE_SIZE - 1)) }

#[cfg(not(feature = "swap"))]
pub fn copy_args(cfg: &mut BootConfig) {
    // Copy the args list to target RAM
    cfg.init_size += cfg.args.size();
    let runtime_arg_buffer = cfg.get_top();
    unsafe {
        #[allow(clippy::cast_ptr_alignment)]
        memcpy(runtime_arg_buffer, cfg.args.base as *const usize, cfg.args.size() as usize)
    };
    cfg.args = KernelArguments::new(runtime_arg_buffer);
}

#[derive(Eq, PartialEq)]
enum TagType {
    IniE,
    IniF,
    IniS,
    XKrn,
    Other,
}
impl From<u32> for TagType {
    fn from(code: u32) -> Self {
        if code == u32::from_le_bytes(*b"IniE") {
            TagType::IniE
        } else if code == u32::from_le_bytes(*b"IniF") {
            TagType::IniF
        } else if code == u32::from_le_bytes(*b"IniS") {
            TagType::IniS
        } else if code == u32::from_le_bytes(*b"XKrn") {
            TagType::XKrn
        } else {
            TagType::Other
        }
    }
}
impl TagType {
    #[cfg(feature = "debug-print")]
    pub fn to_str(&self) -> &'static str {
        match self {
            TagType::IniE => "IniE",
            TagType::IniF => "IniF",
            TagType::IniS => "IniS",
            TagType::XKrn => "XKrn",
            TagType::Other => "Other",
        }
    }
}

/// Copy program data from the SPI flash into newly-allocated RAM
/// located at the end of memory space.
fn copy_processes(cfg: &mut BootConfig) {
    let mut _pid = 1;
    for tag in cfg.args.iter() {
        let tag_type = TagType::from(tag.name);
        match tag_type {
            TagType::IniF | TagType::IniE => {
                _pid += 1;
                let mut top = core::ptr::null_mut::<u8>();

                let inie = MiniElf::new(&tag);
                let mut src_paddr =
                    unsafe { cfg.base_addr.add(inie.load_offset as usize / mem::size_of::<usize>()) }
                        as *const u8;

                println!("\n\n{} {} has {} sections", tag_type.to_str(), _pid, inie.sections.len());
                println!(
                    "Initial top: {:x}, extra_pages: {:x}, init_size: {:x}, base_addr: {:x}",
                    cfg.get_top() as *mut u8 as u32,
                    cfg.extra_pages,
                    cfg.init_size,
                    cfg.base_addr as u32
                );

                let mut last_page_vaddr = 0;
                let mut last_section_perfect_fit = false;

                for section in inie.sections.iter() {
                    let flags = section.flags() as u8;
                    // any section that requires "write" must be copied to RAM
                    // note that ELF helpfully adds a 4096-byte gap between non-write pages and write-pages
                    // allowing us to just trundle through the pages and not have to deal with partially
                    // writeable pages.
                    // IniE is always copy_to_ram
                    let copy_to_ram = (flags & MINIELF_FLG_W != 0) || (tag_type == TagType::IniE);

                    if (section.virt as usize) < last_page_vaddr {
                        panic!(
                            "init section addresses are not strictly increasing (new virt: {:08x}, last virt: {:08x})",
                            section.virt, last_page_vaddr
                        );
                    }

                    // cfg.extra_pages tracks how many pages of RAM we've allocated so far
                    // cfg.top() points to the bottom of the most recently allocated page
                    //    - so if cfg.extra_pages is 0, nothing is allocated, and cfg.top() points to
                    //      previously reserved space
                    //
                    // The section length always matches the stride between sections in physical memory.
                    //
                    // However, the section length has nothing to do with the distance between sections in
                    // virtual memory; the virtual start address is allowed to be an
                    // arbitrary number of bytes higher than the previous section end, for
                    // alignment and padding reasons.
                    if copy_to_ram {
                        let mut dst_page_vaddr = section.virt as usize;
                        let mut bytes_to_copy = section.len();

                        if (last_page_vaddr & !(PAGE_SIZE - 1)) != (dst_page_vaddr & !(PAGE_SIZE - 1))
                            || last_section_perfect_fit
                        {
                            // this condition is always true for the first section's first iteration, because
                            // current_vpage_addr starts as NULL; thus we are guaranteed to always
                            // trigger the page allocate/zero mechanism the first time through the loop
                            //
                            // `last_section_perfect_fit` triggers a page allocation as well, because in this
                            // case we had exactly enough data to fill out the
                            // previous section, so we have no more space left in
                            // the current page. We don't automatically allocate a new page because
                            // if it was actually the very last section we *shouldn't* allocate another page;
                            // and we can only know if there's another section
                            // available by dropping off the end of the
                            // loop and coming back to the surrounding for-loop iterator.
                            cfg.extra_pages += 1;
                            top = cfg.get_top() as *mut u8;
                            unsafe {
                                bzero(top, top.add(PAGE_SIZE as usize));
                            }
                        }

                        // Copy the start copying the source data into virtual memory, until the current
                        // page is exhausted.
                        while bytes_to_copy > 0 {
                            let bytes_remaining_in_vpage = PAGE_SIZE - (dst_page_vaddr & (PAGE_SIZE - 1));
                            let copyable_bytes = bytes_remaining_in_vpage.min(bytes_to_copy);
                            last_section_perfect_fit = bytes_remaining_in_vpage == bytes_to_copy;
                            if !section.no_copy() {
                                unsafe {
                                    memcpy(
                                        top.add(dst_page_vaddr & (PAGE_SIZE - 1)),
                                        src_paddr,
                                        copyable_bytes,
                                    );
                                    src_paddr = src_paddr.add(copyable_bytes);
                                }
                            } else {
                                // chunk is already zeroed, because we zeroed the whole page when we got it.
                            }
                            bytes_to_copy -= copyable_bytes;
                            dst_page_vaddr += copyable_bytes;

                            if copyable_bytes == bytes_remaining_in_vpage && bytes_to_copy > 0 {
                                // we've reached the end of the vpage, and there's more to copy:
                                // grab a new page
                                cfg.extra_pages += 1;
                                top = cfg.get_top() as *mut u8;
                                if bytes_to_copy < PAGE_SIZE {
                                    // pre-zero out the page if the remaining data won't fill it.
                                    unsafe {
                                        bzero(top, top.add(PAGE_SIZE as usize));
                                    }
                                }
                            }
                        }
                        // set the vpage based on our current vpage. This allows us to allocate
                        // a new vpage on the next iteration in case there is surprise padding in
                        // the section load address.
                        last_page_vaddr = dst_page_vaddr;
                    } else {
                        top = cfg.get_top() as *mut u8;
                        // forward the FLASH address pointer by the length of the section.
                        src_paddr = unsafe { src_paddr.add(section.len()) };
                    }

                    if VDBG {
                        println!("Looping to the next section");
                        println!(
                            "top: {:x}, extra_pages: {:x}, init_size: {:x}, base_addr: {:x}",
                            cfg.get_top() as *mut u8 as u32,
                            cfg.extra_pages,
                            cfg.init_size,
                            cfg.base_addr as u32
                        );
                        println!("last_page_vaddr: {:x}", last_page_vaddr);
                    }
                }
                println!("Done with sections");
            }
            TagType::IniS => {
                // if swap is not enabled, don't pull this code in, to keep the bootloader light-weight
                #[cfg(feature = "swap")]
                if let Some(swap) = cfg.swap_hal.as_mut() {
                    // IniS does not necessarily exist in linear memory space, so it requires special
                    // handling. Instead of copying the IniS data into RAM, it's copied
                    // into encrypted swap (e.g. the RAM area (again, not necessarily in
                    // linear space) reserved for swap processes).

                    /*
                    Example of an IniS section:
                    1    IniS: entrypoint @ 00021e68, loaded from 00001114.  Sections:
                    Physical offset in swap source image                     Destination range in virtual memory
                             src_swap_img_addr                                          dst_page_vaddr
                                  |                                                              |
                                  v                                                              v
                    Loaded from 00001114 - Section .gcc_except_table   4056 bytes loading into 00010114..000110ec flags: NONE
                    Loaded from 000020ec - Section .rodata        19080 bytes loading into 000110f0..00015b78 flags: NONE
                    Loaded from 00006b74 - Section .eh_frame_hdr   2172 bytes loading into 00015b78..000163f4 flags: EH_HEADER
                    Loaded from 000073f0 - Section .eh_frame       7740 bytes loading into 000163f4..00018230 flags: EH_FRAME
                    Loaded from 0000922c - Section .text          67428 bytes loading into 00019230..00029994 flags: EXECUTE
                    Loaded from 00019990 - Section .data              4 bytes loading into 0002a994..0002a998 flags: WRITE
                    Loaded from 00019994 - Section .sdata            32 bytes loading into 0002a998..0002a9b8 flags: WRITE
                    Loaded from 000199b4 - Section .sbss             64 bytes loading into 0002a9b8..0002a9f8 flags: WRITE | NOCOPY
                    Loaded from 000199f4 - Section .bss             532 bytes loading into 0002a9f8..0002ac0c flags: WRITE | NOCOPY

                    Note that we have full control over what swap block we put things into, but the swap block's
                    address offsets should have a 1:1 correlation to the *virtual* destination addresess. We track
                    the current swap page with `working_page_swap_offset`.
                    */

                    _pid += 1;
                    let mut working_page_swap_offset: Option<usize> = None;
                    let mut working_buf = [0u8; 4096];
                    let mut working_buf_dirty = false;

                    println!("tag size: {:x}", tag.size);
                    println!("tag data: {:x?}", &tag.data);
                    let inis = MiniElf::new(&tag);
                    let mut src_swap_img_addr = inis.load_offset as usize;

                    println!("\n\n{} {} has {} sections", tag_type.to_str(), _pid, inis.sections.len());
                    println!("Swap free page at swap addr: {:x}", cfg.swap_free_page,);

                    let mut last_copy_vaddr = 0;

                    for section in inis.sections.iter() {
                        let mut dst_page_vaddr = section.virt as usize;
                        let mut bytes_to_copy = section.len();

                        if let Some(swap_offset) = working_page_swap_offset {
                            if (last_copy_vaddr & !(PAGE_SIZE - 1)) != (dst_page_vaddr & !(PAGE_SIZE - 1)) {
                                // handle case that the new section destination address is outside of the
                                // current page
                                swap.encrypt_swap_to(
                                    &mut working_buf,
                                    swap_offset * 0x1000,
                                    dst_page_vaddr & !(PAGE_SIZE - 1),
                                    _pid,
                                );
                                cfg.map_swap(swap_offset * 0x1000, dst_page_vaddr & !(PAGE_SIZE - 1), _pid);
                                working_buf.fill(0);
                                working_page_swap_offset = Some(cfg.swap_free_page);
                                working_buf_dirty = false;
                                cfg.swap_free_page += 1;
                            }
                        } else {
                            // very first time through the loop. working_buf is guaranteed to be zero.
                            working_page_swap_offset = Some(cfg.swap_free_page);
                            cfg.swap_free_page += 1;
                        }

                        // Decrypt the source image data and re-encrypt it to swap for the section at hand.
                        //   - dst_page_vaddr is the virtual address of the section. We only care about this
                        //     for tracking offsets in pages, at this stage.
                        //   - working_page_swap_offset is the current destination swap RAM page
                        //   - src_swap_img_addr is the offset of the section in source swap FLASH.
                        //   - no_copy sections need to set the corresponding bytes in swap RAM to zero.
                        //
                        //
                        while bytes_to_copy > 0 {
                            // here are the cases we have to handle:
                            //   - the available decrypted data is larger than the target region to encrypt
                            //   - the available decrypted data is smaller than the target region to encrypt
                            //   - the available decrypted data is equal to the target region to encrypt
                            let src_swap_img_page = src_swap_img_addr & !(PAGE_SIZE - 1);
                            let src_swap_img_offset = src_swap_img_addr & (PAGE_SIZE - 1);
                            // it's almost free to check, so we check at every loop start
                            if swap.decrypt_page_addr() != src_swap_img_page {
                                swap.decrypt_src_page_at(src_swap_img_page);
                            }
                            let decrypt_avail = remaining_in_page(src_swap_img_addr);
                            let dst_page_avail = remaining_in_page(dst_page_vaddr);
                            let dst_page_offset = dst_page_vaddr & (PAGE_SIZE - 1);
                            let copyable = if decrypt_avail >= dst_page_avail {
                                dst_page_avail.min(bytes_to_copy)
                            } else {
                                decrypt_avail.min(bytes_to_copy)
                            };
                            if !section.no_copy() {
                                working_buf[dst_page_offset..dst_page_offset + copyable].copy_from_slice(
                                    &swap.buf_as_ref()[src_swap_img_offset..src_swap_img_offset + copyable],
                                );
                                working_buf_dirty = true;
                            } else {
                                // do nothing, because working_buff is filled with 0 on alloc
                                // but, mark the buffer as dirty, because, it still needs to be committed
                                working_buf_dirty = true;
                            }
                            bytes_to_copy -= copyable;
                            dst_page_vaddr += copyable;
                            src_swap_img_addr += copyable;
                            // if we filled up the destination, grab another page.
                            if (dst_page_vaddr & (PAGE_SIZE - 1)) == 0 {
                                // we copied exactly dst_page_avail, causing us to wrap around to 0
                                // write the existing page to swap, and allocate a new swap page
                                swap.encrypt_swap_to(
                                    &mut working_buf,
                                    working_page_swap_offset.unwrap() * 0x1000,
                                    dst_page_vaddr & !(PAGE_SIZE - 1),
                                    _pid,
                                );
                                cfg.map_swap(
                                    working_page_swap_offset.take().unwrap() * 0x1000,
                                    dst_page_vaddr & !(PAGE_SIZE - 1),
                                    _pid,
                                );
                                working_buf.fill(0);
                                working_page_swap_offset = Some(cfg.swap_free_page);
                                working_buf_dirty = false;
                                cfg.swap_free_page += 1;
                            }
                        }
                        // set the vpage based on our current vpage. This allows us to allocate
                        // a new vpage on the next iteration in case there is surprise padding in
                        // the section load address.
                        last_copy_vaddr = dst_page_vaddr;

                        if SDBG {
                            println!("Looping to the next section (swap)");
                            println!(
                                "  swap_free_page: {:x}, dst_page_vaddr: {:x}, src_swap_img_addr: {:x}",
                                cfg.swap_free_page, dst_page_vaddr, src_swap_img_addr,
                            );
                            println!("  last_copy_vaddr: {:x}", last_copy_vaddr);
                        }
                    }
                    // flush the encryption buffer
                    if working_buf_dirty {
                        swap.encrypt_swap_to(
                            &mut working_buf,
                            working_page_swap_offset.unwrap() * 0x1000,
                            last_copy_vaddr & !(PAGE_SIZE - 1),
                            _pid,
                        );
                        cfg.map_swap(
                            working_page_swap_offset.take().unwrap() * 0x1000,
                            last_copy_vaddr & !(PAGE_SIZE - 1),
                            _pid,
                        );
                    } else {
                        // we didn't use the current page, de-allocate it
                        cfg.swap_free_page -= 1;
                    }
                    println!("Done with sections");
                }
            }
            TagType::XKrn => {
                let prog = unsafe { &*(tag.data.as_ptr() as *const ProgramDescription) };

                // TEXT SECTION
                // Round it off to a page boundary
                let load_size_rounded = (prog.text_size as usize + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
                cfg.extra_pages += load_size_rounded / PAGE_SIZE;
                let top = cfg.get_top();
                println!("\n\nKernel top: {:x}, extra_pages: {:x}", top as u32, cfg.extra_pages);
                unsafe {
                    // Copy the program to the target address, rounding it off to the load size.
                    let src_addr = cfg.base_addr.add(prog.load_offset as usize / mem::size_of::<usize>());
                    println!(
                        "    Copying TEXT from {:08x}-{:08x} to {:08x}-{:08x} ({} bytes long)",
                        src_addr as usize,
                        src_addr as u32 + prog.text_size,
                        top as usize,
                        top as u32 + prog.text_size + 4,
                        prog.text_size + 4
                    );
                    println!(
                        "    Zeroing out TEXT from {:08x}-{:08x}",
                        top.add(prog.text_size as usize / mem::size_of::<usize>()) as usize,
                        top.add(load_size_rounded as usize / mem::size_of::<usize>()) as usize,
                    );

                    memcpy(top, src_addr, prog.text_size as usize + 1);

                    // Zero out the remaining data.
                    bzero(
                        top.add(prog.text_size as usize / mem::size_of::<usize>()),
                        top.add(load_size_rounded as usize / mem::size_of::<usize>()),
                    )
                };

                // DATA SECTION
                // Round it off to a page boundary
                let load_size_rounded =
                    ((prog.data_size + prog.bss_size) as usize + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
                cfg.extra_pages += load_size_rounded / PAGE_SIZE;
                let top = cfg.get_top();
                unsafe {
                    // Copy the program to the target address, rounding it off to the load size.
                    let src_addr = cfg
                        .base_addr
                        .add((prog.load_offset + prog.text_size + 4) as usize / mem::size_of::<usize>() - 1);
                    println!(
                        "    Copying DATA from {:08x}-{:08x} to {:08x}-{:08x} ({} bytes long)",
                        src_addr as usize,
                        src_addr as u32 + prog.data_size,
                        top as usize,
                        top as u32 + prog.data_size,
                        prog.data_size
                    );
                    memcpy(top, src_addr, prog.data_size as usize + 1);

                    // Zero out the remaining data.
                    println!(
                        "    Zeroing out DATA from {:08x} - {:08x}",
                        top.add(prog.data_size as usize / mem::size_of::<usize>()) as usize,
                        top.add(load_size_rounded as usize / mem::size_of::<usize>()) as usize
                    );
                    bzero(
                        top.add(prog.data_size as usize / mem::size_of::<usize>()),
                        top.add(load_size_rounded as usize / mem::size_of::<usize>()),
                    )
                }
            }
            _ => {}
        }
    }
}

unsafe fn memcpy<T>(dest: *mut T, src: *const T, count: usize)
where
    T: Copy,
{
    if VDBG {
        println!(
            "COPY (align {}): {:08x} - {:08x} {} {:08x} - {:08x}",
            mem::size_of::<T>(),
            src as usize,
            src as usize + count,
            count,
            dest as usize,
            dest as usize + count
        );
    }
    core::ptr::copy_nonoverlapping(src, dest, count / mem::size_of::<T>());
}
