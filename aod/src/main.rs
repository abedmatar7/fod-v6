mod panel_writes;
mod power_state;

use panel_writes::PanelWrites;
use power_state::PowerState;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

// Verified Xiaomi 12X (psyche, SM8250-AC, 4.19 kernel) paths
const BL_PATH: &str = "/sys/class/backlight/panel0-backlight/brightness";
const DISP_PARAM_PATH: &str = "/sys/class/drm/card0-DSI-1/disp_param";
const FOD_UI_READY_PATH: &str = "/sys/class/drm/card0-DSI-1/fod_ui_ready";
const XIAOMI_TOUCH_DEV: &str = "/dev/xiaomi-touch";
const EXTCMD_BIN: &str = "/vendor/bin/psyche_extcmd";

// Verified SM8250 mi-dsi-panel disp_param values from dmesg
const DISP_PARAM_FOD_BL_FLAG_ON: &str = "0x1000000\n";
const DISP_PARAM_FOD_BL_FLAG_OFF: &str = "0x2000000\n";
const DISP_PARAM_HBM_FOD_ON: &str = "0x20000\n";
const DISP_PARAM_HBM_FOD_TO_NORM: &str = "0x30000\n";
const DISP_PARAM_FOD_UNLOCK_SUCCESS: &str = "0x70000\n";

// Default backlight levels (overridable via persist.sys.psyche.aod_bl / fp_bl)
const DEFAULT_AOD_BL: u32 = 28;
const DEFAULT_FP_BL: u32 = 100;

// Linux input_event constants (aarch64)
const EV_KEY: u16 = 1;
const KEY_INFO: u16 = 0x166; // Xiaomi SM8250 FOD key (358)
const BTN_INFO: u16 = 0x152; // Fallback FOD key (338)

// Xiaomi touch ioctl constants
const TOUCH_IOC_SET_CUR_VALUE_IO: u64 = 0x5400; // _IO('T', 0)
const TOUCH_IOC_SET_CUR_VALUE_IOW: u64 = 0x40085400; // _IOW('T', 0, int[2])
const TOUCH_FOD_ENABLE: i32 = 10;
const TOUCH_FOD_ICON_ENABLE: i32 = 16;

// EVIOCGNAME(128) = _IOC(_IOC_READ, 'E', 0x06, 128) = 0x80804506
const EVIOCGNAME_128: u64 = 0x80804506;

extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
}

struct ExtCmdPipe {
    _child: Child,
    stdin: ChildStdin,
}

static WRITES: PanelWrites = PanelWrites::new();
static IS_DOZING: AtomicBool = AtomicBool::new(false);
static FOD_ACTIVE: AtomicBool = AtomicBool::new(false);
static HBM_IS_ON: AtomicBool = AtomicBool::new(false);
static EXTCMD_PIPE: Mutex<Option<ExtCmdPipe>> = Mutex::new(None);

fn setprop(key: &str, val: &str) {
    let _ = Command::new("/system/bin/setprop")
        .args([key, val])
        .status();
}

fn getprop(key: &str) -> String {
    Command::new("/system/bin/getprop")
        .arg(key)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn getprop_u32(key: &str, fallback: u32) -> u32 {
    getprop(key).parse::<u32>().unwrap_or(fallback)
}

fn disp_param_enabled() -> bool {
    let v = getprop("persist.sys.psyche.fod_disp_param");
    v.is_empty() || v == "1" || v.eq_ignore_ascii_case("true")
}

/// One-time startup check so the first unlock after boot knows if Keyguard auth is already active.
fn query_initial_fod_active() -> bool {
    if let Ok(out) = Command::new("/system/bin/dumpsys")
        .arg("fingerprint")
        .output()
    {
        if let Ok(text) = String::from_utf8(out.stdout) {
            for line in text.lines() {
                if line.contains("Current operation:")
                    && (line.contains("FingerprintAuthenticationClient")
                        || line.contains("FingerprintEnrollClient")
                        || line.contains("FingerprintDetectClient"))
                {
                    return true;
                }
            }
        }
    }
    false
}

fn send_goodix_extcmd(cmd: u32, param: u32) {
    if !Path::new(EXTCMD_BIN).exists() {
        return;
    }
    let line = format!("{} {}\n", cmd, param);
    if let Ok(mut guard) = EXTCMD_PIPE.lock() {
        let mut needs_spawn = guard.is_none();
        if let Some(ref mut pipe) = *guard {
            if pipe.stdin.write_all(line.as_bytes()).is_err() || pipe.stdin.flush().is_err() {
                needs_spawn = true;
            }
        }
        if needs_spawn {
            *guard = None;
            if let Ok(mut child) = Command::new(EXTCMD_BIN)
                .arg("--pipe")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(line.as_bytes());
                    let _ = stdin.flush();
                    *guard = Some(ExtCmdPipe { _child: child, stdin });
                }
            }
        }
    }
}

fn wait_for_fod_hbm_ready() {
    // fod_ui_ready is a bitmask: 1 = icon shown, 3 (bit 1 set) = HBM committed on panel.
    // Wait until bit 1 is set (up to 150ms), then let 1-2 frames settle at 120Hz before Goodix captures.
    for _ in 0..30 {
        if let Ok(s) = std::fs::read_to_string(FOD_UI_READY_PATH) {
            if let Ok(val) = s.trim().parse::<u32>() {
                if (val & 2) != 0 {
                    thread::sleep(Duration::from_millis(20));
                    return;
                }
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    // Fallback settle margin if kernel didn't expose bit 1
    thread::sleep(Duration::from_millis(25));
}

fn enable_xiaomi_touch_fod() {
    if let Ok(file) = OpenOptions::new().read(true).write(true).open(XIAOMI_TOUCH_DEV) {
        let fd = file.as_raw_fd();
        for mode in [TOUCH_FOD_ENABLE, TOUCH_FOD_ICON_ENABLE] {
            let mut arg: [i32; 2] = [mode, 1];
            unsafe {
                let rc = ioctl(fd, TOUCH_IOC_SET_CUR_VALUE_IO, arg.as_mut_ptr());
                if rc != 0 {
                    let _ = ioctl(fd, TOUCH_IOC_SET_CUR_VALUE_IOW, arg.as_mut_ptr());
                }
            }
        }
    }
}

fn set_fod_hbm(gen: u64, enable: bool, unlock_success: bool) {
    if !disp_param_enabled() || !Path::new(DISP_PARAM_PATH).exists() {
        return;
    }
    if enable {
        HBM_IS_ON.store(true, Ordering::SeqCst);
        let _ = WRITES.apply(gen, DISP_PARAM_PATH, DISP_PARAM_FOD_BL_FLAG_ON);
        let _ = WRITES.apply(gen, DISP_PARAM_PATH, DISP_PARAM_HBM_FOD_ON);
        wait_for_fod_hbm_ready();
        // cmd=4 (NIT FOD) + cmd=10 (HBM and Green spot ready -> unblocks Goodix capture semaphore)
        send_goodix_extcmd(4, 1);
        send_goodix_extcmd(10, 1);
    } else if HBM_IS_ON.swap(false, Ordering::SeqCst) || unlock_success {
        // Only issue off sequence if HBM was actually on (or unlock just succeeded)
        send_goodix_extcmd(10, 0);
        send_goodix_extcmd(4, 0);
        if unlock_success {
            let _ = WRITES.apply(gen, DISP_PARAM_PATH, DISP_PARAM_FOD_UNLOCK_SUCCESS);
        }
        let _ = WRITES.apply(gen, DISP_PARAM_PATH, DISP_PARAM_HBM_FOD_TO_NORM);
        let _ = WRITES.apply(gen, DISP_PARAM_PATH, DISP_PARAM_FOD_BL_FLAG_OFF);
    }
}

fn find_fts_event_device() -> String {
    for i in 0..16 {
        let path = format!("/dev/input/event{}", i);
        if let Ok(file) = File::open(&path) {
            let mut buf = [0u8; 128];
            let rc = unsafe { ioctl(file.as_raw_fd(), EVIOCGNAME_128, buf.as_mut_ptr()) };
            if rc > 0 {
                let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                if let Ok(name) = std::str::from_utf8(&buf[..end]) {
                    if name == "fts" || name.contains("fts") {
                        return path;
                    }
                }
            }
        }
    }
    "/dev/input/event5".to_string()
}

fn spawn_fod_input_watcher(aod_bl: u32, fp_bl: u32) {
    thread::spawn(move || {
        loop {
            enable_xiaomi_touch_fod();
            let dev_path = find_fts_event_device();
            let mut file = match File::open(&dev_path) {
                Ok(f) => f,
                Err(_) => {
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };

            let mut ev = [0u8; 24];
            loop {
                if file.read_exact(&mut ev).is_err() {
                    break;
                }
                let ev_type = u16::from_ne_bytes([ev[16], ev[17]]);
                let ev_code = u16::from_ne_bytes([ev[18], ev[19]]);
                let ev_val = i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]);

                if ev_type == EV_KEY && (ev_code == KEY_INFO || ev_code == BTN_INFO) {
                    if ev_val == 1 {
                        // Gate FOD HBM strictly on active fingerprint session (or AOD dozing)
                        if !FOD_ACTIVE.load(Ordering::SeqCst) && !IS_DOZING.load(Ordering::SeqCst) {
                            continue;
                        }
                        let gen = WRITES.invalidate();
                        if IS_DOZING.load(Ordering::SeqCst) {
                            let bl_str = format!("{}\n", fp_bl);
                            let _ = WRITES.apply(gen, BL_PATH, &bl_str);
                        }
                        set_fod_hbm(gen, true, false);
                    } else if ev_val == 0 {
                        if HBM_IS_ON.load(Ordering::SeqCst) {
                            let gen = WRITES.invalidate();
                            set_fod_hbm(gen, false, false);
                            if IS_DOZING.load(Ordering::SeqCst) {
                                let bl_str = format!("{}\n", aod_bl);
                                let _ = WRITES.apply(gen, BL_PATH, &bl_str);
                            }
                        }
                    }
                }
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
}

fn wait_for_backlight_node() {
    for _ in 0..60 {
        if Path::new(BL_PATH).exists() {
            return;
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn is_fod_session_start(line: &str) -> bool {
    if line.contains("BiometricScheduler")
        && line.contains("[Started]")
        && (line.contains("FingerprintAuthenticationClient")
            || line.contains("FingerprintEnrollClient")
            || line.contains("FingerprintDetectClient"))
    {
        return true;
    }
    if line.contains("BiometricStateCallback")
        && (line.contains("state updated to 1")
            || line.contains("state updated to 2")
            || line.contains("state updated to 3")
            || line.contains("state updated to 4"))
    {
        return true;
    }
    if line.contains("mShowForEnroll:true")
        || (line.contains("showFingerprintIcon") && !line.contains("does not meet"))
        || line.contains("ENROLL_ENROLLING")
    {
        return true;
    }
    false
}

fn is_fod_session_end(line: &str) -> bool {
    if line.contains("Client finished, state updated to 0")
        || line.contains("No operations, returning to idle")
        || line.contains("mShowForEnroll:false")
        || (line.contains("hideFingerprintIcon") && !line.contains("does not meet"))
    {
        return true;
    }
    false
}

fn main() {
    setprop("ro.nees.fod.compat", "1");
    setprop("persist.sys.psyche.aod_keep_doze", "1");

    wait_for_backlight_node();
    enable_xiaomi_touch_fod();

    // Check if Keyguard already started auth before psyche_aodd launched
    FOD_ACTIVE.store(query_initial_fod_active(), Ordering::SeqCst);

    // Warm up psyche_extcmd pipe once at boot
    send_goodix_extcmd(10, 0);
    send_goodix_extcmd(4, 0);

    let aod_bl = getprop_u32("persist.sys.psyche.aod_bl", DEFAULT_AOD_BL);
    let fp_bl = getprop_u32("persist.sys.psyche.fp_bl", DEFAULT_FP_BL);

    spawn_fod_input_watcher(aod_bl, fp_bl);

    let mut current_state = PowerState::Unknown;

    loop {
        let mut child = match Command::new("/system/bin/logcat")
            .args(["-b", "all", "-v", "brief", "-T", "1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => {
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        };

        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            for line_res in reader.lines() {
                let line = match line_res {
                    Ok(l) => l,
                    Err(_) => break,
                };

                // Track active FOD enroll/auth sessions
                if is_fod_session_start(&line) {
                    FOD_ACTIVE.store(true, Ordering::SeqCst);
                } else if line.contains("onAuthenticated") {
                    FOD_ACTIVE.store(false, Ordering::SeqCst);
                    let gen = WRITES.invalidate();
                    set_fod_hbm(gen, false, true);
                } else if is_fod_session_end(&line) {
                    FOD_ACTIVE.store(false, Ordering::SeqCst);
                    if HBM_IS_ON.load(Ordering::SeqCst) {
                        let gen = WRITES.invalidate();
                        set_fod_hbm(gen, false, false);
                    }
                }

                // Track PowerState transitions
                if let Some(new_state) = PowerState::parse_from_log_line(&line) {
                    if new_state != current_state {
                        current_state = new_state;
                        IS_DOZING.store(current_state == PowerState::Dozing, Ordering::SeqCst);
                        let gen = WRITES.invalidate();
                        match current_state {
                            PowerState::Dozing => {
                                enable_xiaomi_touch_fod();
                                let bl_str = format!("{}\n", aod_bl);
                                let _ = WRITES.apply(gen, BL_PATH, &bl_str);
                            }
                            PowerState::Awake => {
                                enable_xiaomi_touch_fod();
                                set_fod_hbm(gen, false, false);
                            }
                            PowerState::Asleep => {
                                set_fod_hbm(gen, false, false);
                            }
                            PowerState::Unknown => {}
                        }
                    }
                }
            }
        }

        let _ = child.wait();
        thread::sleep(Duration::from_secs(1));
    }
}
