//! Reader thread for the fader HID device.
//!
//! The device emits a vendor-page report (page 0xFF00, report id 2) of up to
//! eight signed 16-bit LE axes carrying raw wiper mV (~0..3300). Matching is
//! transport-agnostic — USB and BLE (BlueZ HOG) both surface as hidraw — and
//! keys off the vendor usage first (BLE may present a different PID), same as
//! the Windows sibling:
//!   1. usage page 0xFF00 + usage 0x0001, preferring VID 0x1d50;
//!   2. any device on the vendor page, preferring VID 0x1d50.
//!
//! Access relies on the packaged udev uaccess rule
//! (packaging/udev/70-fadewire.rules).

use std::sync::mpsc::Sender;
use std::time::Duration;

use fadewire_core::MAX_AXES;
use hidapi::{DeviceInfo, HidApi, HidDevice};

const VID: u16 = 0x1d50;
const REPORT_ID: u8 = 0x02;
const RETRY: Duration = Duration::from_millis(1500);

pub enum HidEvent {
    /// One fader report: `axes[..count]` are valid.
    Axes { axes: [i32; MAX_AXES], count: usize },
    /// Connection state changed (deduplicated).
    Status { connected: bool, message: String },
}

pub fn spawn(tx: Sender<HidEvent>) {
    std::thread::Builder::new()
        .name("fadewire-hid".into())
        .spawn(move || run(tx))
        .expect("spawning the hid thread");
}

fn run(tx: Sender<HidEvent>) {
    let mut last_status = String::new();

    let mut api = loop {
        match HidApi::new() {
            Ok(api) => break api,
            Err(e) => {
                send_status(&tx, &mut last_status, false, &format!("hid init failed: {e}"));
                std::thread::sleep(RETRY);
            }
        }
    };

    loop {
        let _ = api.refresh_devices();
        let found = find_fader(&api).map(|d| {
            (
                d.path().to_owned(),
                d.product_string().unwrap_or("fader device").to_string(),
            )
        });
        let Some((path, name)) = found else {
            send_status(
                &tx,
                &mut last_status,
                false,
                "fader device not found — plug it in (is the udev rule installed?)",
            );
            std::thread::sleep(RETRY);
            continue;
        };

        let dev = match api.open_path(&path) {
            Ok(dev) => dev,
            Err(e) => {
                send_status(
                    &tx,
                    &mut last_status,
                    false,
                    &format!("found {name}, but couldn't open it: {e}"),
                );
                std::thread::sleep(RETRY);
                continue;
            }
        };

        send_status(&tx, &mut last_status, true, &format!("connected to {name}"));
        if read_loop(&dev, &tx).is_err() {
            return; // engine gone — exit the thread
        }
        send_status(&tx, &mut last_status, false, "device disconnected — rescanning");
    }
}

/// Read reports until the device errors (unplug). `Err` means the receiver
/// hung up and the thread should exit.
fn read_loop(dev: &HidDevice, tx: &Sender<HidEvent>) -> Result<(), ()> {
    let mut buf = [0u8; 64];
    loop {
        match dev.read_timeout(&mut buf, 1000) {
            Ok(0) => continue, // timeout — keep listening
            Ok(n) if n >= 3 && buf[0] == REPORT_ID => {
                let count = ((n - 1) / 2).min(MAX_AXES);
                let mut axes = [0i32; MAX_AXES];
                for (i, axis) in axes.iter_mut().enumerate().take(count) {
                    *axis = i16::from_le_bytes([buf[1 + 2 * i], buf[2 + 2 * i]]) as i32;
                }
                tx.send(HidEvent::Axes { axes, count }).map_err(|_| ())?;
            }
            Ok(_) => continue, // some other report — ignore
            Err(_) => return Ok(()), // device gone — rescan
        }
    }
}

fn find_fader(api: &HidApi) -> Option<&DeviceInfo> {
    let mut usage_any: Option<&DeviceInfo> = None;
    let mut page_ours: Option<&DeviceInfo> = None;
    let mut page_any: Option<&DeviceInfo> = None;
    for d in api.device_list() {
        if d.usage_page() != 0xFF00 {
            continue;
        }
        if d.usage() == 0x0001 {
            if d.vendor_id() == VID {
                return Some(d); // exact match — done
            }
            usage_any.get_or_insert(d);
        } else if d.vendor_id() == VID {
            page_ours.get_or_insert(d);
        } else {
            page_any.get_or_insert(d);
        }
    }
    usage_any.or(page_ours).or(page_any)
}

fn send_status(tx: &Sender<HidEvent>, last: &mut String, connected: bool, message: &str) {
    if last == message {
        return; // don't spam the engine with the same state every retry
    }
    *last = message.to_string();
    let _ = tx.send(HidEvent::Status {
        connected,
        message: message.to_string(),
    });
}
