use crate::{
    subscriber::{self, Shared},
    view::{now, rows, Status},
};
use std::{
    cell::RefCell,
    ptr::{null, null_mut},
    sync::{Arc, Mutex},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::*, WindowsAndMessaging::*},
};
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}
fn notify(label: &str) -> bool {
    use windows::{
        core::HSTRING,
        Data::Xml::Dom::XmlDocument,
        UI::Notifications::{NotificationSetting, ToastNotification, ToastNotificationManager},
    };
    let send = || -> windows::core::Result<bool> {
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(
            "AgentMonitor.Dashboard",
        ))?;
        if notifier.Setting()? != NotificationSetting::Enabled {
            return Ok(false);
        }
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(format!("<toast><visual><binding template=\"ToastGeneric\"><text>Agent Monitor</text><text>{label}</text></binding></visual></toast>")))?;
        notifier.Show(&ToastNotification::CreateToastNotification(&xml)?)?;
        Ok(true)
    };
    send().unwrap_or(false)
}
struct Ui {
    state: Arc<Mutex<Shared>>,
    summary: HWND,
    coverage: HWND,
    filter: HWND,
    check: HWND,
    list: HWND,
    last: Vec<Vec<String>>,
    font: HFONT,
    title_font: HFONT,
}
thread_local! {static UI:RefCell<Option<Ui>>=const{RefCell::new(None)};}
unsafe fn control(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    font: HFONT,
) -> HWND {
    let hwnd = CreateWindowExW(
        0,
        wide(class).as_ptr(),
        wide(text).as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        x,
        y,
        w,
        h,
        parent,
        id as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
    SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
    hwnd
}
unsafe fn font(size: i32, weight: i32) -> HFONT {
    CreateFontW(
        -size,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        1,
        0,
        0,
        5,
        0,
        wide("Segoe UI").as_ptr(),
    )
}
unsafe fn refresh(u: &mut Ui) {
    let filter = SendMessageW(u.filter, CB_GETCURSEL, 0, 0);
    let time = now();
    let state = u.state.lock().unwrap();
    let mut data = Vec::new();
    let mut counts = [0usize; 4];
    let mut coverage = vec![];
    if let Some(f) = &state.feed.frame {
        for s in &f.sources {
            coverage.push(format!(
                "{} {} · {}",
                s.mode.replace('_', " "),
                s.version.as_deref().unwrap_or("?"),
                s.coverage
            ));
        }
        for row in rows(f, time) {
            match row.status {
                Status::Running => counts[0] += 1,
                Status::Input => counts[1] += 1,
                Status::Completed => counts[2] += 1,
                Status::Unknown => counts[3] += 1,
                _ => (),
            }
            if (filter == 1 && row.status != Status::Input)
                || (filter == 2 && row.status != Status::Running)
                || (filter == 3 && row.status != Status::Unknown)
            {
                continue;
            }
            let marker = match row.status {
                Status::Input => "◆",
                Status::Running => "▶",
                Status::Completed => "✓",
                Status::Interrupted => "■",
                Status::Idle => "○",
                Status::Unknown => "?",
            };
            data.push(vec![
                format!("{marker}  {}", row.status.name()),
                row.source.project.clone(),
                row.id[row.id.len() - 8..].into(),
                format!(
                    "{}{}",
                    row.source.mode.replace('_', " "),
                    if row.copies > 1 { " (+)" } else { "" }
                ),
                row.confidence.into(),
                format!("{}s", time.saturating_sub(row.data.observed_ms) / 1000),
                row.data.visibility.clone(),
            ]);
        }
    }
    let connected = state.feed.frame.is_some();
    let failed = state.notification_failed;
    drop(state);
    let caption = if connected {
        format!("Connected     •     {} running       {} input needed       {} completed       {} uncertain",counts[0],counts[1],counts[2],counts[3])
    } else {
        "Disconnected     •     Waiting for a fresh collector snapshot".into()
    };
    SetWindowTextW(u.summary, wide(&caption).as_ptr());
    let footer = format!(
        "Windows   ·   {}{}",
        coverage.join("     |     "),
        if failed {
            "     |     Desktop notifications unavailable"
        } else {
            ""
        }
    );
    SetWindowTextW(u.coverage, wide(&footer).as_ptr());
    if data == u.last {
        return;
    }
    SendMessageW(u.list, WM_SETREDRAW, 0, 0);
    for (i, row) in data.iter().enumerate() {
        for (j, cell) in row.iter().enumerate() {
            if u.last.get(i).and_then(|r| r.get(j)) == Some(cell) {
                continue;
            }
            let mut t = wide(cell);
            let mut item: LVITEMW = std::mem::zeroed();
            item.mask = LVIF_TEXT;
            item.iItem = i as i32;
            item.iSubItem = j as i32;
            item.pszText = t.as_mut_ptr();
            let insert = j == 0 && i >= u.last.len();
            SendMessageW(
                u.list,
                if insert {
                    LVM_INSERTITEMW
                } else {
                    LVM_SETITEMTEXTW
                },
                i,
                &item as *const _ as isize,
            );
        }
    }
    for i in (data.len()..u.last.len()).rev() {
        SendMessageW(u.list, LVM_DELETEITEM, i, 0);
    }
    u.last = data;
    SendMessageW(u.list, WM_SETREDRAW, 1, 0);
    InvalidateRect(u.list, null(), 1);
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: usize, l: isize) -> isize {
    match msg {
        WM_TIMER | WM_COMMAND => {
            UI.with_borrow_mut(|ui| {
                if let Some(u) = ui {
                    if msg == WM_COMMAND && l as HWND == u.check {
                        let mut s = u.state.lock().unwrap();
                        s.enabled = SendMessageW(u.check, BM_GETCHECK, 0, 0) == 1;
                        s.notification_failed = false;
                    }
                    refresh(u)
                }
            });
            0
        }
        WM_SIZE => {
            UI.with_borrow(|ui| {
                if let Some(u) = ui {
                    let width = (l as u32 & 65535) as i32;
                    let height = ((l as u32) >> 16) as i32;
                    MoveWindow(
                        u.list,
                        20,
                        145,
                        (width - 40).max(1),
                        (height - 204).max(1),
                        1,
                    );
                    MoveWindow(
                        u.coverage,
                        20,
                        (height - 51).max(145),
                        (width - 40).max(1),
                        48,
                        1,
                    );
                }
            });
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
pub fn run() {
    unsafe {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let port = args
            .first()
            .and_then(|s| s.to_str()?.parse::<u16>().ok())
            .filter(|p| *p != 0);
        if port.is_none() || args.len() != 2 {
            MessageBoxW(
                null_mut(),
                wide("Usage: agent-monitor-dashboard.exe PORT PRIVATE_API_DIRECTORY").as_ptr(),
                wide("Agent Monitor").as_ptr(),
                MB_OK,
            );
            return;
        }
        let h = GetModuleHandleW(null());
        let name = wide("AgentMonitorDashboard");
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: h,
            lpszClassName: name.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
            ..std::mem::zeroed()
        };
        RegisterClassW(&class);
        InitCommonControls();
        let hwnd = CreateWindowExW(
            0,
            name.as_ptr(),
            wide("Agent Monitor").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1010,
            535,
            null_mut(),
            null_mut(),
            h,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        let normal = font(16, 400);
        let title = font(30, 650);
        control(
            hwnd,
            "STATIC",
            "Agent Monitor",
            0,
            0,
            20,
            14,
            310,
            42,
            title,
        );
        let summary = control(hwnd, "STATIC", "Connecting…", 0, 0, 21, 63, 940, 25, normal);
        let filter = control(
            hwnd,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32 | WS_TABSTOP,
            1,
            20,
            102,
            180,
            170,
            normal,
        );
        for s in ["All sessions", "Input needed", "Running", "Uncertain"] {
            SendMessageW(filter, CB_ADDSTRING, 0, wide(s).as_ptr() as isize);
        }
        SendMessageW(filter, CB_SETCURSEL, 0, 0);
        let check = control(
            hwnd,
            "BUTTON",
            "Desktop notifications",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            2,
            220,
            100,
            215,
            30,
            normal,
        );
        let list = control(
            hwnd,
            "SysListView32",
            "",
            LVS_REPORT | LVS_SINGLESEL | WS_TABSTOP,
            3,
            20,
            145,
            950,
            280,
            normal,
        );
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            0,
            (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize,
        );
        for (i, (caption, width)) in [
            ("Activity / last turn", 175),
            ("Project", 145),
            ("Session", 94),
            ("Observation mode", 165),
            ("Confidence", 116),
            ("Age", 58),
            ("Visibility", 90),
        ]
        .iter()
        .enumerate()
        {
            let mut t = wide(caption);
            let mut col: LVCOLUMNW = std::mem::zeroed();
            col.mask = LVCF_TEXT | LVCF_WIDTH;
            col.cx = *width;
            col.pszText = t.as_mut_ptr();
            SendMessageW(list, LVM_INSERTCOLUMNW, i, &col as *const _ as isize);
        }
        let coverage = control(hwnd, "STATIC", "", 0, 0, 20, 435, 950, 48, normal);
        let state = Arc::new(Mutex::new(Shared::default()));
        subscriber::start(port.unwrap(), args[1].clone().into(), state.clone(), notify);
        UI.with_borrow_mut(|ui| {
            *ui = Some(Ui {
                state,
                summary,
                coverage,
                filter,
                check,
                list,
                last: vec![],
                font: normal,
                title_font: title,
            })
        });
        SetTimer(hwnd, 1, 100, None);
        ShowWindow(hwnd, SW_SHOW);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        UI.with_borrow_mut(|ui| {
            if let Some(u) = ui.take() {
                DeleteObject(u.font);
                DeleteObject(u.title_font);
            }
        });
    }
}
