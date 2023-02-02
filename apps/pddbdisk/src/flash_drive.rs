use pddb::PddbKey;
use std::io::{Read, Seek, Write};
use xous::MessageEnvelope;

struct LBA<'a> {
    pub index: usize,
    pub pddb_key: pddb::PddbKey<'a>,
}

pub struct FlashDrive<'a> {
    capacity: usize,
    block_size: usize,
    pddb: pddb::Pddb,

    lba_cache: uluru::LRUCache<LBA<'a>, 20>,
}

#[derive(Debug)]
pub enum FlashDriveError {
    CapacityNotPageAligned,
}

impl<'a> FlashDrive<'a> {
    pub fn new(capacity: usize, block_size: usize) -> Result<Self, FlashDriveError> {
        if (capacity % 1024) != 0 {
            return Err(FlashDriveError::CapacityNotPageAligned);
        }

        let ret = Self {
            capacity,
            block_size,
            pddb: pddb::Pddb::new(),
            lba_cache: uluru::LRUCache::default(),
        };

        Ok(ret)
    }

    pub fn read(&mut self, msg: &mut MessageEnvelope) {
        let body = msg
            .body
            .memory_message_mut()
            .expect("incorrect message type received");
        let lba = body.offset.map(|v| v.get()).unwrap_or_default();
        let data = body.buf.as_slice_mut::<u8>();

        self.read_inner(lba, data);
    }

    pub fn write(&mut self, msg: &mut MessageEnvelope) {
        let body = msg
            .body
            .memory_message_mut()
            .expect("incorrect message type received");
        let lba = body.offset.map(|v| v.get()).unwrap_or_default();
        let data = body.buf.as_slice_mut::<u8>();

        self.write_inner(lba, data);
    }

    pub fn max_lba(&self, msg: &mut MessageEnvelope) {
        xous::return_scalar(msg.sender, self.max_lba_inner() as usize).unwrap();
    }
}

impl FlashDrive<'_> {
    fn key(&self, lba: usize) -> PddbKey {
        let mut lba_buf = itoa::Buffer::new();
        let key = self
            .pddb
            .get(
                "pddbdisk",
                lba_buf.format(lba as u32),
                None,
                true,
                true,
                None,
                None::<fn()>,
            )
            .unwrap();

        key
    }

    fn read_inner(&mut self, lba: usize, data: &mut [u8]) {
        //     let maybe_lba = self.lba_cache.find(|candidate| candidate.index == lba);
        //     let key = match maybe_lba {
        //         Some(lba) => {
        //             lba.pddb_key.read(data).unwrap();
        //             return
        //         }
        //         None => {
        //             let mut lba_buf = itoa::Buffer::new();
        //             let mut key = self.pddb
        //                 .get("pddbdisk", lba_buf.format(lba as u32), None, true, true, None, None::<fn()>)
        //                 .unwrap();

        //                 key.read(data).unwrap();

        //             Some(LBA { index: lba, pddb_key: key })
        //         }
        //     };

        //     self.lba_cache.insert(key.unwrap());
        // }
        self.key(lba).read(data).unwrap();
    }

    fn write_inner(&mut self, lba: usize, data: &mut [u8]) {
        // let maybe_lba = self.lba_cache.find(|candidate| candidate.index == lba);
        // let key = match maybe_lba {
        //     Some(lba) => *lba,
        //     None => {
        //         let mut lba_buf = itoa::Buffer::new();
        //         let key = self.pddb
        //             .get("pddbdisk", lba_buf.format(lba as u32), None, true, true, None, None::<fn()>)
        //             .unwrap();

        //         self.lba_cache.insert(LBA { index: lba, pddb_key: key }).unwrap()
        //     }
        // };

        self.key(lba).write(data).unwrap();
    }

    fn max_lba_inner(&self) -> u32 {
        (self.capacity as u32 / 512) - 1
    }
}
