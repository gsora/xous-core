use core::mem::size_of;

use aes_gcm_siv::{AeadInPlace, Aes256GcmSiv, Error, KeyInit, Nonce, Tag};
use loader::swap::SwapSpec;
use xous::MemoryRange;

pub const PAGE_SIZE: usize = 4096;

/// This defines a set of functions to get and receive MACs (message
/// authentication codes, also referred to as the tag in AES-GCM-SIV.
pub struct SwapHal {
    swap_mem: MemoryRange,
    dst_data_area: &'static mut [u8],
    dst_mac_area: &'static mut [u8],
    cipher: Aes256GcmSiv,
}
impl SwapHal {
    pub fn new(spec: &SwapSpec) -> Self {
        let swap_mem = xous::syscall::map_memory(
            xous::MemoryAddress::new(spec.swap_base as usize),
            None,
            spec.swap_len as usize,
            xous::MemoryFlags::R | xous::MemoryFlags::W,
        )
        .expect("Couldn't map swap area into swapper");
        // compute the MAC area needed for the total RAM size. This is a slight over-estimate
        // because once we remove the MAC area, we need even less storage, but it's a small error.
        let mac_size = (spec.swap_len as usize / 4096) * size_of::<Tag>();
        let mac_size_to_page = (mac_size + (PAGE_SIZE - 1)) & !(PAGE_SIZE - 1);
        let ram_size_actual = (spec.swap_len as usize & !(PAGE_SIZE - 1)) - mac_size_to_page;

        // safety: this is safe because all values within the memory region can be represented in `u8`
        let swap_slice = swap_mem.as_mut_ptr();
        Self {
            swap_mem,
            // safety: the ram swap area is guaranteed aligned by the ram_offset specifier, and our
            // calculations on lengths ensure area alignment
            dst_data_area: unsafe { core::slice::from_raw_parts_mut(swap_slice as *mut u8, ram_size_actual) },
            // safety: the ram swap area is guaranteed aligned by the ram_offset specifier, and our
            // calculations on lengths ensure area alignment
            dst_mac_area: unsafe {
                core::slice::from_raw_parts_mut((swap_slice as *mut u8).add(ram_size_actual), mac_size)
            },
            cipher: Aes256GcmSiv::new((&spec.key).into()),
        }
    }

    /// The data to be encrypted is provided in `buf`, and is replaced with part of the encrypted data upon
    /// completion of the routine.
    pub fn encrypt_swap_to(
        &mut self,
        buf: &mut [u8],
        swap_count: u32,
        dest_offset: usize,
        src_vaddr: usize,
        src_pid: u8,
    ) {
        // println!("enc_to: pid: {}, src_vaddr: {:x} dest_offset: {:x}", src_pid, src_vaddr, dest_offset);
        assert!(buf.len() == PAGE_SIZE);
        assert!(dest_offset & (PAGE_SIZE - 1) == 0);
        let mut nonce = [0u8; size_of::<Nonce>()];
        nonce[0..4].copy_from_slice(&swap_count.to_be_bytes()); // this is the `swap_count` field
        nonce[5] = src_pid;
        let ppage_masked = dest_offset & !(PAGE_SIZE - 1);
        nonce[6..9].copy_from_slice(&(ppage_masked as u32).to_be_bytes()[..3]);
        let vpage_masked = src_vaddr & !(PAGE_SIZE - 1);
        nonce[9..12].copy_from_slice(&(vpage_masked as u32).to_be_bytes()[..3]);
        let aad: &[u8] = &[];
        match self.cipher.encrypt_in_place_detached(Nonce::from_slice(&nonce), aad, buf) {
            Ok(tag) => {
                self.dst_data_area[dest_offset..dest_offset + PAGE_SIZE].copy_from_slice(buf);
                let mac_offset = (dest_offset / PAGE_SIZE) * size_of::<Tag>();
                self.dst_mac_area[mac_offset..mac_offset + size_of::<Tag>()].copy_from_slice(tag.as_slice());
                // println!("Nonce: {:x?}, tag: {:x?}", &nonce, tag.as_slice());
                // println!("dst_mac_area: {:x?}", &self.dst_mac_area[..32]);
            }
            Err(e) => panic!("Encryption error to swap ram: {:?}", e),
        }
    }

    /// Used to examine contents of swap RAM. Decrypted data is returned as a slice.
    pub fn decrypt_swap_from(
        &mut self,
        buf: &mut [u8],
        swap_count: u32,
        src_offset: usize,
        dst_vaddr: usize,
        dst_pid: u8,
    ) -> Result<(), Error> {
        // println!("Decrypt swap:");
        // println!("  offset: {:x}, vaddr: {:x}, pid: {}", src_offset, dst_vaddr, dst_pid);
        assert!(src_offset & (PAGE_SIZE - 1) == 0);
        assert!(buf.len() == PAGE_SIZE);

        let mut nonce = [0u8; size_of::<Nonce>()];
        nonce[0..4].copy_from_slice(&swap_count.to_be_bytes()); // this is the `swap_count` field
        nonce[5] = dst_pid;
        let ppage_masked = src_offset & !(PAGE_SIZE - 1);
        nonce[6..9].copy_from_slice(&(ppage_masked as u32).to_be_bytes()[..3]);
        let vpage_masked = dst_vaddr & !(PAGE_SIZE - 1);
        nonce[9..12].copy_from_slice(&(vpage_masked as u32).to_be_bytes()[..3]);
        let aad: &[u8] = &[];
        let mut tag = [0u8; size_of::<Tag>()];
        let mac_offset = (src_offset / PAGE_SIZE) * size_of::<Tag>();
        tag.copy_from_slice(&self.dst_mac_area[mac_offset..mac_offset + size_of::<Tag>()]);
        // println!("dst_mac_area: {:x?}", &self.dst_mac_area[..32]);
        buf.copy_from_slice(&self.dst_data_area[src_offset..src_offset + PAGE_SIZE]);
        // println!("Nonce: {:x?}, tag: {:x?}", &nonce, &tag);
        self.cipher.decrypt_in_place_detached(Nonce::from_slice(&nonce), aad, buf, (&tag).into())
    }
}
