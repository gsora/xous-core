use crate::{ShellCmdApi, CommonEnv};
use usb_device_xous::{UsbHid, UsbDeviceState, UsbDeviceType};
use std::fmt::Write;

#[derive(Debug)]
pub struct Usb {
    usb_dev: UsbHid,
}
impl Usb {
    pub fn new() -> Usb {
        Usb {
            usb_dev: UsbHid::new(),
        }
    }
}

impl<'a> ShellCmdApi<'a> for Usb {
    cmd_api!(usb); // inserts boilerplate for command API

    fn process(&mut self, args: xous_ipc::String::<1024>, _env: &mut CommonEnv) -> Result<Option<xous_ipc::String::<1024>>, xous::Error> {
        let mut ret = xous_ipc::String::<1024>::new();
        let helpstring = "usb [hid] [debug] [send <string>] [status] [leds] [lock] [unlock] [kbdtest]";

        let mut tokens = args.as_str().unwrap().split(' ');

        if let Some(sub_cmd) = tokens.next() {
            match sub_cmd {
                "hid" => {
                    self.usb_dev.switch_to_core(usb_device_xous::UsbDeviceType::Hid).unwrap();
                    write!(ret, "USB connected to HID core").unwrap();
                }
                "debug" => {
                    self.usb_dev.switch_to_core(usb_device_xous::UsbDeviceType::Debug).unwrap();
                    write!(ret, "USB connected to Debug core").unwrap();
                }
                "send" => {
                    match self.usb_dev.get_current_core() {
                        Ok(UsbDeviceType::Hid) => {
                            let mut val = String::new();
                            join_tokens(&mut val, &mut tokens);
                            match self.usb_dev.send_str(&val) {
                                Ok(n) => write!(ret, "Sent {} chars", n).unwrap(),
                                Err(_e) => write!(ret, "Can't send: are we connected to a host?").unwrap(),
                            }
                        }
                        Ok(UsbDeviceType::Debug) => {
                            write!(ret, "HID core not connected: please issue 'usb hid' first").unwrap();
                        }
                        _ => write!(ret, "Invalid response checking status").unwrap(),
                    }
                }
                "kbdtest" => {
                    let mut test_str = String::new();
                    for c in 0x20..0x7F { // includes a space as the first character
                        // safety - the bounds are checked above in the loop to be the printable ASCII character range.
                        test_str.push(unsafe{char::from_u32_unchecked(c as u32)});
                    }
                    test_str.push('\n');
                    match self.usb_dev.get_current_core() {
                        Ok(UsbDeviceType::Hid) => {
                            match self.usb_dev.send_str(&test_str) {
                                Ok(n) => write!(ret, "Sent {} test string", n).unwrap(),
                                Err(_e) => write!(ret, "Can't send: are we connected to a host?").unwrap(),
                            }
                        }
                        Ok(UsbDeviceType::Debug) => {
                            write!(ret, "HID core not connected: please issue 'usb hid' first").unwrap();
                        }
                        _ => write!(ret, "Invalid response checking status").unwrap(),
                    }
                }
                "status" => {
                    match self.usb_dev.get_current_core() {
                        Ok(UsbDeviceType::Debug) => write!(ret, "Debug core connected").unwrap(),
                        Ok(UsbDeviceType::Hid) => {
                            match self.usb_dev.status() {
                                UsbDeviceState::Configured => write!(ret, "HID core connected to host").unwrap(),
                                UsbDeviceState::Suspend => write!(ret, "HID in suspend").unwrap(),
                                _ => write!(ret, "HID not connected to USB host").unwrap(),
                            }
                        }
                        _ => write!(ret, "Invalid response checking status").unwrap(),
                    }
                }
                "leds" => {
                    match self.usb_dev.get_current_core() {
                        Ok(UsbDeviceType::Hid) => {
                            match self.usb_dev.get_led_state() {
                                Ok(leds) => write!(ret, "LEDs: {:?}", leds).unwrap(),
                                _ => write!(ret, "Not connected to USB host or other error").unwrap(),
                            }
                        }
                        Ok(UsbDeviceType::Debug) => {
                            write!(ret, "HID core not connected: please issue 'usb hid' first").unwrap();
                        }
                        _ => write!(ret, "Invalid response checking status").unwrap(),
                    }
                }
                "lock" => {
                    self.usb_dev.restrict_debug_access(true).unwrap();
                    write!(ret, "USB debug port locked out; one word at 0x80000000 is disclosable via USB.").unwrap();
                }
                "unlock" => {
                    self.usb_dev.restrict_debug_access(false).unwrap();
                    write!(ret, "USB debug port unlocked: all secrets are readable via USB!").unwrap();
                }
                _ => {
                    write!(ret, "{}", helpstring).unwrap();
                }
            }

        } else {
            write!(ret, "{}", helpstring).unwrap();
        }
        Ok(Some(ret))
    }
}

fn join_tokens<'a>(buf: &mut String, tokens: impl Iterator<Item = &'a str>) {
    for (i, tok) in tokens.enumerate() {
        if i == 0 {
            write!(buf, "{}", tok).unwrap();
        } else {
            write!(buf, " {}", tok).unwrap();
        }
    }
}
