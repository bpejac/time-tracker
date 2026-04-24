use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{DateTime, Duration, Local, Utc};
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, WebviewWindowBuilder};

// ── Persistent state (survives reboots) ───────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct SavedState {
    paused: bool,
}

// ── JSONL log entry ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct LogEntry {
    ts: DateTime<Utc>,
    event: String,
    reason: String,
}

// ── Runtime tracker ───────────────────────────────────────────────────────────

struct Tracker {
    paused: bool,
    session_start: Option<DateTime<Utc>>,
    today_secs: i64,
    today_date: String,
    data_dir: PathBuf,
    // Closure that updates the tray menu; captures cloned MenuItem handles.
    refresh_menu: Box<dyn Fn(bool, i64) + Send + 'static>,
}

impl Tracker {
    fn today_key() -> String {
        Local::now().format("%Y-%m-%d").to_string()
    }

    fn total_today_secs(&self) -> i64 {
        let in_session = self
            .session_start
            .map(|s| (Utc::now() - s).num_seconds().max(0))
            .unwrap_or(0);
        self.today_secs + in_session
    }

    fn start(&mut self, reason: &str) {
        if self.session_start.is_some() {
            return;
        }
        self.session_start = Some(Utc::now());
        self.append_log("work_start", reason);
        self.do_refresh();
    }

    fn stop(&mut self, reason: &str) {
        let Some(start) = self.session_start.take() else {
            return;
        };
        self.today_secs += (Utc::now() - start).num_seconds().max(0);
        self.append_log("work_stop", reason);
        self.do_refresh();
    }

    fn pause(&mut self) {
        self.paused = true;
        self.stop("user_pause");
        self.save_state();
    }

    fn resume(&mut self) {
        self.paused = false;
        self.start("user_resume");
        self.save_state();
    }

    pub fn on_screen_lock(&mut self) {
        if !self.paused {
            self.stop("screen_lock");
        }
    }

    pub fn on_screen_unlock(&mut self) {
        if !self.paused {
            self.start("screen_unlock");
        }
    }

    fn log_path(&self) -> PathBuf {
        self.data_dir.join(format!("{}.jsonl", self.today_date))
    }

    fn state_path(&self) -> PathBuf {
        self.data_dir.join("state.json")
    }

    fn append_log(&self, event: &str, reason: &str) {
        let entry = LogEntry {
            ts: Utc::now(),
            event: event.to_string(),
            reason: reason.to_string(),
        };
        if let Ok(line) = serde_json::to_string(&entry) {
            if let Ok(mut f) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.log_path())
            {
                let _ = writeln!(f, "{}", line);
            }
        }
    }

    fn save_state(&self) {
        if let Ok(json) = serde_json::to_string(&SavedState { paused: self.paused }) {
            let _ = fs::write(self.state_path(), json);
        }
    }

    fn do_refresh(&self) {
        (self.refresh_menu)(self.paused, self.total_today_secs());
    }
}

// ── Global singleton ──────────────────────────────────────────────────────────

static TRACKER: OnceLock<Arc<Mutex<Tracker>>> = OnceLock::new();

fn tracker() -> &'static Arc<Mutex<Tracker>> {
    TRACKER.get().expect("tracker not initialized")
}

// ── macOS screen lock via CoreFoundation (no objc2 dependency) ────────────────

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void};

    type CFNotificationCenterRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFIndex = isize;
    type CFNotificationSuspensionBehavior = CFIndex;

    // kCFNotificationSuspensionBehaviorDeliverImmediately
    const DELIVER_IMMEDIATELY: CFNotificationSuspensionBehavior = 4;
    const UTF8: u32 = 0x0800_0100;

    type Callback = unsafe extern "C" fn(
        CFNotificationCenterRef,
        *mut c_void,
        CFStringRef,
        *const c_void,
        *const c_void,
    );

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFNotificationCenterGetDistributedCenter() -> CFNotificationCenterRef;
        fn CFNotificationCenterAddObserver(
            center: CFNotificationCenterRef,
            observer: *const c_void,
            callback: Callback,
            name: CFStringRef,
            object: *const c_void,
            suspension_behavior: CFNotificationSuspensionBehavior,
        );
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: *const c_void);
    }

    unsafe extern "C" fn on_lock(
        _: CFNotificationCenterRef,
        _: *mut c_void,
        _: CFStringRef,
        _: *const c_void,
        _: *const c_void,
    ) {
        if let Ok(mut t) = super::tracker().lock() {
            t.on_screen_lock();
        }
    }

    unsafe extern "C" fn on_unlock(
        _: CFNotificationCenterRef,
        _: *mut c_void,
        _: CFStringRef,
        _: *const c_void,
        _: *const c_void,
    ) {
        if let Ok(mut t) = super::tracker().lock() {
            t.on_screen_unlock();
        }
    }

    pub fn register_screen_observer() {
        unsafe {
            let center = CFNotificationCenterGetDistributedCenter();
            // Use a non-null dummy pointer as the observer identifier.
            let dummy = 1usize as *const c_void;

            for (name_bytes, cb) in [
                (b"com.apple.screenIsLocked\0".as_ptr(), on_lock as Callback),
                (
                    b"com.apple.screenIsUnlocked\0".as_ptr(),
                    on_unlock as Callback,
                ),
            ] {
                let cf_name = CFStringCreateWithCString(
                    std::ptr::null(),
                    name_bytes as *const c_char,
                    UTF8,
                );
                CFNotificationCenterAddObserver(
                    center,
                    dummy,
                    cb,
                    cf_name,
                    std::ptr::null(),
                    DELIVER_IMMEDIATELY,
                );
                CFRelease(cf_name);
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn compute_today_secs(data_dir: &PathBuf, today: &str) -> i64 {
    let log_path = data_dir.join(format!("{}.jsonl", today));
    let content = match fs::read_to_string(&log_path) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    let mut total = 0i64;
    let mut last_start: Option<DateTime<Utc>> = None;

    for line in content.lines() {
        if let Ok(entry) = serde_json::from_str::<LogEntry>(line) {
            match entry.event.as_str() {
                "work_start" => last_start = Some(entry.ts),
                "work_stop" => {
                    if let Some(start) = last_start.take() {
                        total += (entry.ts - start).num_seconds().max(0);
                    }
                }
                _ => {}
            }
        }
    }
    total
}

fn fmt_duration(total_secs: i64) -> String {
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    format!("{}h {:02}m", h, m)
}

// ── History data structures ───────────────────────────────────────────────────

#[derive(Serialize)]
struct TimeSegment {
    start: String,
    end: Option<String>,
}

#[derive(Serialize)]
struct DayHistory {
    date: String,
    segments: Vec<TimeSegment>,
}

fn read_day_segments(data_dir: &PathBuf, date: &str) -> Vec<TimeSegment> {
    let content = match fs::read_to_string(data_dir.join(format!("{}.jsonl", date))) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut segments = Vec::new();
    let mut last_start: Option<DateTime<Utc>> = None;

    for line in content.lines() {
        if let Ok(entry) = serde_json::from_str::<LogEntry>(line) {
            match entry.event.as_str() {
                "work_start" => last_start = Some(entry.ts),
                "work_stop" => {
                    if let Some(start) = last_start.take() {
                        segments.push(TimeSegment {
                            start: start.to_rfc3339(),
                            end: Some(entry.ts.to_rfc3339()),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    if let Some(start) = last_start {
        segments.push(TimeSegment {
            start: start.to_rfc3339(),
            end: None,
        });
    }

    segments
}

#[tauri::command]
fn get_history() -> Vec<DayHistory> {
    let (data_dir, today) = {
        let t = tracker().lock().unwrap();
        (t.data_dir.clone(), t.today_date.clone())
    };

    (0i64..7)
        .rev()
        .map(|days_ago| {
            let date = (Local::now() - Duration::days(days_ago))
                .format("%Y-%m-%d")
                .to_string();
            let is_today = date == today;
            let mut segments = read_day_segments(&data_dir, &date);

            if is_today {
                let now = Utc::now().to_rfc3339();
                for seg in segments.iter_mut() {
                    if seg.end.is_none() {
                        seg.end = Some(now.clone());
                    }
                }
            }

            DayHistory { date, segments }
        })
        .collect()
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_history])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let data_dir = app.path().app_data_dir()?;
            fs::create_dir_all(&data_dir)?;

            let saved: SavedState = fs::read_to_string(data_dir.join("state.json"))
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            let today = Tracker::today_key();
            let today_secs = compute_today_secs(&data_dir, &today);

            let status_text = if saved.paused { "⏸ Paused" } else { "● Tracking" };
            let toggle_text = if saved.paused { "Resume" } else { "Pause" };

            let status_item =
                MenuItem::with_id(app, "status", status_text, false, None::<&str>)?;
            let today_item = MenuItem::with_id(
                app,
                "today",
                format!("Today: {}", fmt_duration(today_secs)),
                false,
                None::<&str>,
            )?;
            let toggle_item =
                MenuItem::with_id(app, "toggle", toggle_text, true, None::<&str>)?;
            let history_item =
                MenuItem::with_id(app, "history", "View History", true, None::<&str>)?;

            let menu = Menu::with_items(
                app,
                &[
                    &status_item,
                    &today_item,
                    &PredefinedMenuItem::separator(app)?,
                    &toggle_item,
                    &PredefinedMenuItem::separator(app)?,
                    &history_item,
                    &PredefinedMenuItem::separator(app)?,
                    // &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
                ],
            )?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => {
                        let mut t = tracker().lock().unwrap();
                        if t.paused {
                            t.resume();
                        } else {
                            t.pause();
                        }
                    }
                    "history" => {
                        if let Some(win) = app.get_webview_window("history") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        } else if let Ok(win) = WebviewWindowBuilder::new(
                            app,
                            "history",
                            tauri::WebviewUrl::App("history.html".into()),
                        )
                        .title("Time Tracker — History")
                        .inner_size(920.0, 560.0)
                        .resizable(true)
                        .build()
                        {
                            let win2 = win.clone();
                            win.on_window_event(move |event| {
                                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                                    api.prevent_close();
                                    let _ = win2.hide();
                                }
                            });
                        }
                    }
                    "quit" => {
                        {
                            let mut t = tracker().lock().unwrap();
                            t.stop("quit");
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            // Clone menu items into the refresh closure so it can update them
            // from any thread without going through the tray handle.
            let s_item = status_item.clone();
            let t_item = today_item.clone();
            let g_item = toggle_item.clone();
            let refresh_fn = Box::new(move |paused: bool, total_secs: i64| {
                let _ = s_item.set_text(if paused { "⏸ Paused" } else { "● Tracking" });
                let _ = t_item.set_text(format!("Today: {}", fmt_duration(total_secs)));
                let _ = g_item.set_text(if paused { "Resume" } else { "Pause" });
            });

            let tracker_arc = Arc::new(Mutex::new(Tracker {
                paused: saved.paused,
                session_start: None,
                today_secs,
                today_date: today,
                data_dir,
                refresh_menu: refresh_fn,
            }));

            TRACKER
                .set(tracker_arc.clone())
                .map_err(|_| "tracker already initialized")?;

            {
                let mut t = tracker_arc.lock().unwrap();
                if !t.paused {
                    t.start("startup");
                }
                t.do_refresh();
            }

            #[cfg(target_os = "macos")]
            macos::register_screen_observer();

            // Refresh "Today" display every minute.
            let t_clone = tracker_arc.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
                if let Ok(t) = t_clone.lock() {
                    t.do_refresh();
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
