/// The `time_server` is unique is that it is written for exclusive use by `libstd` to extract time.
///
/// It also has a single hook that is callable from the PDDB to initialize a time value once the
/// PDDB itself has been initialized. Because time initialization breaks several abstractions, the
/// system is forced to reboot after time is initialized.
///
/// Q: why don't we integrate this into the ticktimer?
/// A: The ticktimer must be (1) lightweight and (2) used as a dependency by almost everything.
///    Pulling this functionality into the ticktimer both makes it heavier, and also more importantly,
///    creates circular dependencies on `llio` and `pddb`.
///
/// System time is composed of:
///    "hardware `u64`"" + "offset to RT" -> SystemTime
/// "offset to RT" is composed of:
///   - offset to UTC
///   - offset to current TZ
/// "hardware `u64`" composed of:
///   - the current number of seconds counted by the RTC module
///   *or*
///   - the number of seconds counted by the RTC module at time T + ticktimer offset since T
/// The second representation is an optimization to avoid hitting the I2C module constantly to
/// read RTC, plus you get milliseconds resolution. Time "T" can be updated at any time by just
/// reading the RTC and noting the ticktimer offset at the point of reading.
use std::thread;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use llio::*;
use pddb::{Pddb, PddbMountPoller};
use std::io::{Read, Write, Seek, SeekFrom};
use num_traits::*;

/// This is a "well known name" used by `libstd` to connect to the time server
/// Even thought it is "public" nobody connects to it directly, they connect to it via `libstd`
/// hence the scope of the name is private to this crate.
pub const TIME_SERVER_PUBLIC: &'static [u8; 16] = b"timeserverpublic";

/// Dictionary for RTC settings.
pub(crate) const TIME_SERVER_DICT: &'static str = "sys.rtc";
/// This is the UTC offset from the current hardware RTC reading. This should be fixed once time is set.
const TIME_SERVER_UTC_OFFSET: &'static str = "utc_offset";
/// This is the offset from UTC to the display time zone. This can vary when the user changes time zones.
pub(crate) const TIME_SERVER_TZ_OFFSET: &'static str = "tz_offset";

const CTL3: usize = 0;
const SECS: usize = 1;
const MINS: usize = 2;
const HOURS: usize = 3;
const DAYS: usize = 4;
// note 5 is skipped - this is weekdays, and is unused
const MONTHS: usize = 6;
const YEARS: usize = 7;

/// Do not modify the discriminants in this structure. They are used in `libstd` directly.
#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub(crate) enum TimeOp {
    /// Sync offsets to hardware RTC
    HwSync = 0,
    /// Suspend/resume call
    SusRes = 1,
    /// Indicates the current time is precisely the provided number of ms since EPOCH
    SetUtcTimeMs = 2,
    /// Get UTC time in ms since EPOCH
    GetUtcTimeMs = 3,
    /// Get local time in ms since EPOCH
    GetLocalTimeMs = 4,
    /// Sets the timezone offset, in milliseconds.
    SetTzOffsetMs = 5,
    /// Query to see if timezone and time relative to UTC have been set.
    WallClockTimeInit = 6,
    /// Self-poll for PDDB mount
    PddbMountPoll = 7,
}

/// Do not modify the discriminants in this structure. They are used in `libstd` directly.
#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub(crate) enum PrivTimeOp {
    /// Reset the hardware RTC count
    ResetRtc = 0,
}

pub fn start_time_server() {
    let rtc_checked = Arc::new(AtomicBool::new(false));

    // this thread handles reading & updating the time offset from the PDDB
    thread::spawn({
        let rtc_checked = rtc_checked.clone();
        move || {
            // the public SID is well known and accessible by anyone who uses `libstd`
            let pub_sid = xous::create_server_with_address(&TIME_SERVER_PUBLIC)
                .expect("Couldn't create Ticktimer server");
            let xns = xous_names::XousNames::new().unwrap();
            let llio = llio::Llio::new(&xns);

            let mut utc_offset_ms = 0i64;
            let mut tz_offset_ms = 0i64;
            let tt = ticktimer_server::Ticktimer::new().unwrap();
            // this routine can't proceed until the RTC has passed its power-on sanity checks
            while !rtc_checked.load(Ordering::SeqCst) {
                tt.sleep_ms(42).unwrap();
            }
            let mut start_rtc_secs = llio.get_rtc_secs().expect("couldn't read RTC offset value");
            let mut start_tt_ms = tt.elapsed_ms();
            log::trace!("start_rtc_secs: {}", start_rtc_secs);
            log::trace!("start_tt_ms: {}", start_tt_ms);

            // register a suspend/resume listener
            let sr_cid = xous::connect(pub_sid).expect("couldn't create suspend callback connection");
            let mut susres = susres::Susres::new(
                Some(susres::SuspendOrder::Early),
                &xns,
                TimeOp::SusRes as u32,
                sr_cid
            ).expect("couldn't create suspend/resume object");
            let self_cid = xous::connect(pub_sid).unwrap();
            let pddb_poller = PddbMountPoller::new();
            // enqueue a the first mount poll message
            xous::send_message(self_cid,
                xous::Message::new_scalar(TimeOp::PddbMountPoll.to_usize().unwrap(), 0, 0, 0, 0)
            ).expect("couldn't check mount poll");
            // an initial behavior which just retuns the raw RTC time, until the PDDB is mounted.
            let mut temp = 0;
            loop {
                if pddb_poller.is_mounted_nonblocking() {
                    log::debug!("PDDB mount detected, transitioning to real-time adjusted server");
                    break;
                }
                let msg = xous::receive_message(pub_sid).unwrap();
                match FromPrimitive::from_usize(msg.body.id()) {
                    Some(TimeOp::PddbMountPoll) => {
                        tt.sleep_ms(330).unwrap();
                        if temp < 10 {
                            log::trace!("mount poll");
                        }
                        temp += 1;
                        xous::send_message(self_cid,
                            xous::Message::new_scalar(TimeOp::PddbMountPoll.to_usize().unwrap(), 0, 0, 0, 0)
                        ).expect("couldn't check mount poll");
                    }
                    Some(TimeOp::SusRes) => xous::msg_scalar_unpack!(msg, token, _, _, _, {
                        susres.suspend_until_resume(token).expect("couldn't execute suspend/resume");
                        // resync time on resume
                        start_rtc_secs = llio.get_rtc_secs().expect("couldn't read RTC offset value");
                        start_tt_ms = tt.elapsed_ms();
                    }),
                    Some(TimeOp::HwSync) => {
                        start_rtc_secs = llio.get_rtc_secs().expect("couldn't read RTC offset value");
                        start_tt_ms = tt.elapsed_ms();
                    },
                    Some(TimeOp::GetUtcTimeMs) => xous::msg_blocking_scalar_unpack!(msg, _, _, _, _, {
                        let t =
                            start_rtc_secs as i64 * 1000i64
                            + (tt.elapsed_ms() - start_tt_ms) as i64;
                        log::debug!("hw only UTC ms {}", t);
                        xous::return_scalar2(msg.sender,
                            (((t as u64) >> 32) & 0xFFFF_FFFF) as usize,
                            (t as u64 & 0xFFFF_FFFF) as usize,
                        ).expect("couldn't respond to GetUtcTimeMs");
                    }),
                    Some(TimeOp::GetLocalTimeMs) => xous::msg_blocking_scalar_unpack!(msg, _, _, _, _, {
                        let t =
                            start_rtc_secs as i64 * 1000i64
                            + (tt.elapsed_ms() - start_tt_ms) as i64;
                        assert!(t > 0, "time result is negative, this is an error");
                        log::debug!("hw only local ms {}", t);
                        xous::return_scalar2(msg.sender,
                            (((t as u64) >> 32) & 0xFFFF_FFFF) as usize,
                            (t as u64 & 0xFFFF_FFFF) as usize,
                        ).expect("couldn't respond to GetLocalTimeMs");
                    }),
                    Some(TimeOp::WallClockTimeInit) => xous::msg_blocking_scalar_unpack!(msg, _, _, _, _, {
                        // definitely not initialized
                        xous::return_scalar(msg.sender, 0).unwrap();
                    }),
                    _ => log::warn!("Time server can't handle this message yet: {:?}", msg),
                }
            }
            // once the PDDB is mounted, read in the time zone offsets and then restart the
            // loop handler using the offsets.
            let mut offset_handle = Pddb::new();
            let mut offset_key = offset_handle.get(
                TIME_SERVER_DICT,
                TIME_SERVER_UTC_OFFSET,
                None, true, true,
                Some(8),
                None::<fn()>
            ).expect("couldn't open UTC offset key");
            let mut tz_handle = Pddb::new();
            let mut tz_key = tz_handle.get(
                TIME_SERVER_DICT,
                TIME_SERVER_TZ_OFFSET,
                None, true, true,
                Some(8),
                None::<fn()>
            ).expect("couldn't open TZ offset key");
            let mut offset_buf = [0u8; 8];
            if offset_key.read(&mut offset_buf).unwrap_or(0) == 8 {
                utc_offset_ms = i64::from_le_bytes(offset_buf);
            } // else 0 is the error value, so leave it at that.
            let mut tz_buf = [0u8; 8];
            if tz_key.read(&mut tz_buf).unwrap_or(0) == 8 {
                tz_offset_ms = i64::from_le_bytes(tz_buf);
            }
            log::debug!("offset_key: {}", utc_offset_ms);
            log::debug!("tz_key: {}", tz_offset_ms);
            log::debug!("start_rtc_secs: {}", start_rtc_secs);
            log::debug!("start_tt_ms: {}", start_tt_ms);
            loop {
                let msg = xous::receive_message(pub_sid).unwrap();
                match FromPrimitive::from_usize(msg.body.id()) {
                    Some(TimeOp::PddbMountPoll) => {
                        // do nothing, we're mounted now.
                        continue;
                    },
                    Some(TimeOp::SusRes) => xous::msg_scalar_unpack!(msg, token, _, _, _, {
                        susres.suspend_until_resume(token).expect("couldn't execute suspend/resume");
                        // resync time on resume
                        start_rtc_secs = llio.get_rtc_secs().expect("couldn't read RTC offset value");
                        start_tt_ms = tt.elapsed_ms();
                    }),
                    Some(TimeOp::HwSync) => {
                        start_rtc_secs = llio.get_rtc_secs().expect("couldn't read RTC offset value");
                        start_tt_ms = tt.elapsed_ms();
                    },
                    Some(TimeOp::GetUtcTimeMs) => xous::msg_blocking_scalar_unpack!(msg, _, _, _, _, {
                        let t =
                            start_rtc_secs as i64 * 1000i64
                            + (tt.elapsed_ms() - start_tt_ms) as i64
                            + utc_offset_ms;
                        assert!(t > 0, "time result is negative, this is an error");
                        log::trace!("utc ms {}", t);
                        xous::return_scalar2(msg.sender,
                            (((t as u64) >> 32) & 0xFFFF_FFFF) as usize,
                            (t as u64 & 0xFFFF_FFFF) as usize,
                        ).expect("couldn't respond to GetUtcTimeMs");
                    }),
                    Some(TimeOp::GetLocalTimeMs) => xous::msg_blocking_scalar_unpack!(msg, _, _, _, _, {
                        log::trace!("current offset {}", (start_rtc_secs as i64 * 1000i64 + (tt.elapsed_ms() - start_tt_ms) as i64) / 1000);
                        let t =
                            start_rtc_secs as i64 * 1000i64
                            + (tt.elapsed_ms() - start_tt_ms) as i64
                            + utc_offset_ms
                            + tz_offset_ms;
                        assert!(t > 0, "time result is negative, this is an error");
                        log::trace!("local since epoch {}", t / 1000);
                        xous::return_scalar2(msg.sender,
                            (((t as u64) >> 32) & 0xFFFF_FFFF) as usize,
                            (t as u64 & 0xFFFF_FFFF) as usize,
                        ).expect("couldn't respond to GetLocalTimeMs");
                    }),
                    Some(TimeOp::SetUtcTimeMs) => xous::msg_scalar_unpack!(msg, utc_hi_ms, utc_lo_ms, _, _, {
                        let utc_time_ms = (utc_hi_ms as i64) << 32 | (utc_lo_ms as i64);
                        start_rtc_secs = llio.get_rtc_secs().expect("couldn't read RTC offset value");
                        start_tt_ms = tt.elapsed_ms();
                        log::info!("utc_time: {}", utc_time_ms / 1000);
                        log::info!("rtc_secs: {}", start_rtc_secs);
                        log::info!("start_tt_ms: {}", start_tt_ms);
                        let offset =
                            utc_time_ms -
                            (start_rtc_secs as i64) * 1000;
                        utc_offset_ms = offset;
                        offset_key.seek(SeekFrom::Start(0)).expect("couldn't seek");
                        log::info!("setting offset to {} secs", offset / 1000);
                        assert_eq!(offset_key.write(&offset.to_le_bytes()).unwrap_or(0), 8, "couldn't commit UTC time offset to PDDB");
                        offset_key.flush().expect("couldn't flush PDDB");
                    }),
                    Some(TimeOp::SetTzOffsetMs) => xous::msg_scalar_unpack!(msg, tz_hi_ms, tz_lo_ms, _, _, {
                        let tz_ms = ((tz_hi_ms as i64) << 32) | (tz_lo_ms as i64);
                        // sanity check with very broad bounds: I don't know of any time zones that are more than +/2 days from UTC
                        // 86400 seconds in a day, times 1000 milliseconds, times 2 days
                        if tz_ms < -(86400 * 1000 * 2) ||
                        tz_ms > 86400 * 1000 * 2 {
                            log::warn!("Requested timezone offset {} is out of bounds, ignoring!", tz_ms);
                            continue;
                        } else {
                            tz_offset_ms = tz_ms;
                            tz_key.seek(SeekFrom::Start(0)).expect("couldn't seek");
                            log::info!("setting tz offset to {} secs", tz_ms / 1000);
                            assert_eq!(tz_key.write(&tz_ms.to_le_bytes()).unwrap_or(0), 8, "couldn't commit TZ time offset to PDDB");
                            tz_key.flush().expect("couldn't flush PDDB");
                        }
                    }),
                    Some(TimeOp::WallClockTimeInit) => xous::msg_blocking_scalar_unpack!(msg, _, _, _, _, {
                        if utc_offset_ms == 0 || tz_offset_ms == 0 {
                            xous::return_scalar(msg.sender, 0).unwrap();
                        } else {
                            xous::return_scalar(msg.sender, 1).unwrap();
                        }
                    }),
                    None => log::error!("Time server public thread received unknown opcode: {:?}", msg),
                }
            }
        }
    });

    // this thread handles more sensitive operations on the RTC.
    thread::spawn({
        let rtc_checked = rtc_checked.clone();
        move || {
            let xns = xous_names::XousNames::new().unwrap();
            // we expect exactly one connection from the PDDB
            let priv_sid = xns.register_name(pddb::TIME_SERVER_PDDB, Some(1)).expect("can't register server");
            let mut i2c = llio::I2c::new(&xns);
            let trng = trng::Trng::new(&xns).unwrap();

            // on boot, do the validation checks of the RTC. If it is not initialized or corrupted, fix it.
            let mut settings = [0u8; 8];
            loop {
                match i2c.i2c_read(ABRTCMC_I2C_ADR, ABRTCMC_CONTROL3, &mut settings, None) {
                    Ok(I2cStatus::ResponseReadOk) => break,
                    _ => {
                        log::error!("Couldn't check RTC, retrying!");
                        xous::yield_slice(); // recheck in a fast loop, we really should be able to grab this resource on boot.
                    },
                };
            }
            if is_rtc_invalid(&settings) {
                log::warn!("RTC settings were invalid. Re-initializing! {:?}", settings);
                settings[CTL3] = (Control3::BATT_STD_BL_EN).bits();
                let mut start_time = trng.get_u64().unwrap();
                // set the clock to a random start time from 1 to 10 years into its maximum range of 100 years
                settings[SECS] = to_bcd((start_time & 0xFF) as u8 % 60);
                start_time >>= 8;
                settings[MINS] = to_bcd((start_time & 0xFF) as u8 % 60);
                start_time >>= 8;
                settings[HOURS] = to_bcd((start_time & 0xFF) as u8 % 24);
                start_time >>= 8;
                settings[DAYS] = to_bcd((start_time & 0xFF) as u8 % 28 + 1);
                start_time >>= 8;
                settings[MONTHS] = to_bcd((start_time & 0xFF) as u8 % 12 + 1);
                start_time >>= 8;
                settings[YEARS] = to_bcd((start_time & 0xFF) as u8 % 10 + 1);
                i2c.i2c_write(ABRTCMC_I2C_ADR, ABRTCMC_CONTROL3, &settings).expect("RTC access error");
            }
            rtc_checked.store(true, Ordering::SeqCst);
            loop {
                let msg = xous::receive_message(priv_sid).unwrap();
                match FromPrimitive::from_usize(msg.body.id()) {
                    Some(PrivTimeOp::ResetRtc) => xous::msg_blocking_scalar_unpack!(msg, _, _, _, _, {
                        log::warn!("RTC time reset command received.");
                        settings[CTL3] = (Control3::BATT_STD_BL_EN).bits();
                        let mut start_time = trng.get_u64().unwrap();
                        // set the clock to a random start time from 1 to 10 years into its maximum range of 100 years
                        settings[SECS] = to_bcd((start_time & 0xFF) as u8 % 60);
                        start_time >>= 8;
                        settings[MINS] = to_bcd((start_time & 0xFF) as u8 % 60);
                        start_time >>= 8;
                        settings[HOURS] = to_bcd((start_time & 0xFF) as u8 % 24);
                        start_time >>= 8;
                        settings[DAYS] = to_bcd((start_time & 0xFF) as u8 % 28 + 1);
                        start_time >>= 8;
                        settings[MONTHS] = to_bcd((start_time & 0xFF) as u8 % 12 + 1);
                        start_time >>= 8;
                        settings[YEARS] = to_bcd((start_time & 0xFF) as u8 % 10 + 1);
                        i2c.i2c_write(ABRTCMC_I2C_ADR, ABRTCMC_CONTROL3, &settings).expect("RTC access error");
                        xous::return_scalar(msg.sender, 0).unwrap();
                    }),
                    _ => log::error!("Time server private thread received unknown opcode: {:?}", msg),
                }
            }
        }
    });
}

fn is_rtc_invalid(settings: &[u8]) -> bool {
    ((settings[CTL3] & 0xE0) != (Control3::BATT_STD_BL_EN).bits()) // power switchover setting should be initialized
    || ((settings[SECS] & 0x80) != 0)  // clock integrity should be guaranteed
    || (to_binary(settings[SECS]) > 59)
    || (to_binary(settings[MINS]) > 59)
    || (to_binary(settings[HOURS]) > 23) // 24 hour mode is default and assumed
    || (to_binary(settings[DAYS]) > 31) || (to_binary(settings[DAYS]) == 0)
    || (to_binary(settings[MONTHS]) > 12) || (to_binary(settings[MONTHS]) == 0)
    || (to_binary(settings[YEARS]) > 99)
}

fn to_binary(bcd: u8) -> u8 {
    (bcd & 0xf) + ((bcd >> 4) * 10)
}
fn to_bcd(binary: u8) -> u8 {
    let mut lsd: u8 = binary % 10;
    if lsd > 9 {
        lsd = 9;
    }
    let mut msd: u8 = binary / 10;
    if msd > 9 {
        msd = 9;
    }

    (msd << 4) | lsd
}