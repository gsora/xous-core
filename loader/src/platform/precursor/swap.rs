use core::mem::size_of;

use aes_gcm_siv::{AeadInPlace, Aes256GcmSiv, KeyInit, Nonce, Tag};
use utralib::generated::*;

use super::RAM_BASE;
use crate::bootconfig::BootConfig;
use crate::println;
use crate::swap::*;
use crate::PAGE_SIZE;

pub struct SwapHal {
    src_data_area: &'static [u8],
    src_mac_area: &'static [u8],
    partial_nonce: [u8; 8],
    aad: &'static [u8],
    src_cipher: Aes256GcmSiv,
    dst_data_area: &'static mut [u8],
    dst_mac_area: &'static mut [u8],
    dst_cipher: Aes256GcmSiv,
    buf_addr: usize,
    buf: RawPage,
}

impl SwapHal {
    pub fn new(cfg: &BootConfig) -> Option<SwapHal> {
        if let Some(swap) = cfg.swap {
            // safety: the swap source header is guaranteed to be aligned and initialized in memory
            // by the image creator.
            let ssh: &SwapSourceHeader =
                unsafe { (swap.flash_offset as *const SwapSourceHeader).as_ref().unwrap() };
            let swap_len = ssh.mac_offset as usize;

            // compute the MAC area needed for the total RAM size. This is a slight over-estimate
            // because once we remove the MAC area, we need even less storage, but it's a small error.
            let mac_size = (swap.ram_size as usize / 4096) * size_of::<Tag>();
            let mac_size_to_page = (mac_size + (PAGE_SIZE - 1)) / PAGE_SIZE;
            let ram_size_actual = (swap.ram_size as usize & !(PAGE_SIZE - 1)) - mac_size_to_page;

            // access the hardware TRNG manually to generate a per-boot random key for RAM swap
            let trng = CSR::new(utra::trng_kernel::HW_TRNG_KERNEL_BASE as *mut u32);
            for _ in 0..4 {
                // wait until the urandom port is initialized
                while trng.rf(utra::trng_kernel::URANDOM_VALID_URANDOM_VALID) == 0 {}
                // pull a dummy piece of data
                trng.rf(utra::trng_kernel::URANDOM_URANDOM);
            }
            let mut ram_swap_key = [0u8; 32];
            for word in ram_swap_key.chunks_exact_mut(core::mem::size_of::<u32>()) {
                while trng.rf(utra::trng_kernel::URANDOM_VALID_URANDOM_VALID) == 0 {}
                let r = trng.rf(utra::trng_kernel::URANDOM_URANDOM);
                word.copy_from_slice(&r.to_be_bytes())
            }

            let mut hal = SwapHal {
                // safety: the swap raw array is guaranteed to be correctly aligned by the image maker
                src_data_area: unsafe {
                    core::slice::from_raw_parts((swap.flash_offset as usize + 4096) as *const u8, swap_len)
                },
                // safety: the mac raw array is guaranteed to be correctly aligned by the image maker
                src_mac_area: unsafe {
                    core::slice::from_raw_parts(
                        (ssh.mac_offset + swap.flash_offset + 4096) as *const u8,
                        (swap_len / 4096) * size_of::<Tag>(),
                    )
                },
                partial_nonce: [0u8; 8],
                aad: &ssh.aad[..ssh.aad_len as usize],
                src_cipher: Aes256GcmSiv::new((&swap.key).into()),
                // safety: the ram swap area is guaranteed aligned by the ram_offset specifier, and our
                // calculations on lengths ensure area alignment
                dst_data_area: unsafe {
                    core::slice::from_raw_parts_mut(swap.ram_offset as *mut u8, ram_size_actual)
                },
                // safety: the ram swap area is guaranteed aligned by the ram_offset specifier, and our
                // calculations on lengths ensure area alignment
                dst_mac_area: unsafe {
                    core::slice::from_raw_parts_mut(
                        (swap.ram_offset as usize + ram_size_actual) as *mut u8,
                        mac_size,
                    )
                },
                dst_cipher: Aes256GcmSiv::new(&ram_swap_key.into()),
                buf_addr: 0,
                buf: RawPage { data: [0u8; 4096] },
            };
            hal.partial_nonce.copy_from_slice(&ssh.parital_nonce);
            Some(hal)
        } else {
            None
        }
    }

    pub fn decrypt_src_page_at(&mut self, offset: usize) -> &[u8] {
        assert!((offset & 0xFFF) == 0, "offset is not page-aligned");
        self.buf_addr = offset;
        // println!("data area: {:x?}", &self.src_data_area[..4]);
        // println!("offset: {:x}", offset);
        self.buf.data.copy_from_slice(&self.src_data_area[offset..offset + 4096]);
        let mut nonce = [0u8; size_of::<Nonce>()];
        nonce[..4].copy_from_slice(&(offset as u32).to_be_bytes());
        nonce[4..].copy_from_slice(&self.partial_nonce);
        let tag = &self.src_mac_area
            [(offset / 4096) * size_of::<Tag>()..(offset / 4096) * size_of::<Tag>() + size_of::<Tag>()];
        // println!("nonce: {:x?}", nonce);
        // println!("tag: {:x?}", tag);
        // println!("aad: {:x?}", self.aad);
        // println!("data: {:x?}", &self.buf.data[0..32]);
        match self.src_cipher.decrypt_in_place_detached(
            Nonce::from_slice(&nonce),
            self.aad,
            &mut self.buf.data,
            tag.into(),
        ) {
            Ok(_) => &self.buf.data,
            Err(e) => panic!("Decryption error in swap: {:?}", e),
        }
    }

    pub fn decrypt_page_addr(&self) -> usize { self.buf_addr }

    pub fn buf_as_mut(&mut self) -> &mut [u8] { &mut self.buf.data }

    pub fn buf_as_ref(&self) -> &[u8] { &self.buf.data }

    /// Swap count is fixed at 0 by this routine. The data to be encrypted is provided in
    /// `buf`, and is replaced with part of the encrypted data upon completion of the routine.
    pub fn encrypt_swap_to(&mut self, buf: &mut [u8], dest_offset: usize, src_vaddr: usize, src_pid: u8) {
        println!("enc_to: pid: {}, src_vaddr: {:x} dest_offset: {:x}", src_pid, src_vaddr, dest_offset);
        assert!(buf.len() == PAGE_SIZE);
        assert!(dest_offset & (PAGE_SIZE - 1) == 0);
        let mut nonce = [0u8; size_of::<Nonce>()];
        nonce[0..4].copy_from_slice(&[0u8; 4]); // this is the `swap_count` field
        nonce[5] = src_pid;
        let ppage_masked = dest_offset & !(PAGE_SIZE - 1);
        nonce[6..9].copy_from_slice(&(ppage_masked as u32).to_be_bytes()[..3]);
        let vpage_masked = src_vaddr & !(PAGE_SIZE - 1);
        nonce[9..12].copy_from_slice(&(vpage_masked as u32).to_be_bytes()[..3]);
        let aad: &[u8] = &[];
        match self.src_cipher.encrypt_in_place_detached(Nonce::from_slice(&nonce), aad, buf) {
            Ok(tag) => {
                self.dst_data_area[dest_offset..dest_offset + PAGE_SIZE].copy_from_slice(buf);
                let mac_offset = (dest_offset / PAGE_SIZE) * size_of::<Tag>();
                self.dst_mac_area[mac_offset..mac_offset + size_of::<Tag>()].copy_from_slice(tag.as_slice());
            }
            Err(e) => panic!("Encryption error to swap ram: {:?}", e),
        }
    }

    /// Swap count is fixed at 0 by this routine. The data to be encrypted is
    /// assumed to already be in `self.buf`
    pub fn decrypt_swap_from(&mut self, src_offset: usize, dst_vaddr: usize, dst_pid: u8) -> &[u8] {
        assert!(src_offset & (PAGE_SIZE - 1) == 0);
        let mut nonce = [0u8; size_of::<Nonce>()];
        nonce[0..4].copy_from_slice(&[0u8; 4]); // this is the `swap_count` field
        nonce[5] = dst_pid;
        let ppage_masked = src_offset & !(PAGE_SIZE - 1);
        nonce[6..9].copy_from_slice(&(ppage_masked as u32).to_be_bytes()[..3]);
        let vpage_masked = dst_vaddr & !(PAGE_SIZE - 1);
        nonce[9..12].copy_from_slice(&(vpage_masked as u32).to_be_bytes()[..3]);
        let aad: &[u8] = &[];
        let mut tag = [0u8; size_of::<Tag>()];
        let mac_offset = (src_offset / PAGE_SIZE) * size_of::<Tag>();
        tag.copy_from_slice(&self.dst_mac_area[mac_offset..mac_offset + size_of::<Tag>()]);
        self.buf.data.copy_from_slice(&self.dst_data_area[src_offset..src_offset + PAGE_SIZE]);
        match self.src_cipher.decrypt_in_place_detached(
            Nonce::from_slice(&nonce),
            aad,
            &mut self.buf.data,
            (&tag).into(),
        ) {
            Ok(_) => &self.buf.data,
            Err(e) => panic!("Decryption error from swap ram: {:?}", e),
        }
    }

    /// Grabs a slice of the internal buffer. Useful for re-using the decrypted page
    /// between elements of the bootloader (saving us from redundant decrypt ops),
    /// but extremely unsafe because we have to track the use of this buffer manually.
    pub unsafe fn get_decrypt(&self) -> &[u8] { &self.buf.data }
}
