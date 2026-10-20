//! Put the settings popup next to the tray icon.
//!
//! Wayland doesn't let apps position their own windows, but on KDE the compositor can do it:
//! we load a small KWin script that waits for our window to appear and moves it above (or
//! below, for a top panel) the click position, kept inside the usable screen area. On X11 the
//! window is positioned directly instead. Elsewhere it opens wherever the compositor likes.

const PLUGIN: &str = "x360truewireless-popup-placement";

/// Where to put the popup: the point that was clicked (global coordinates), if known.
#[derive(Debug, Clone, Copy)]
pub struct Anchor {
    pub x: i32,
    pub y: i32,
}

impl Anchor {
    pub fn parse(arg: &str) -> Option<Self> {
        let (x, y) = arg.split_once(',')?;
        Some(Anchor {
            x: x.trim().parse().ok()?,
            y: y.trim().parse().ok()?,
        })
    }

    /// Tray hosts that don't report a position send 0,0.
    fn known(self) -> Option<Self> {
        (self.x != 0 || self.y != 0).then_some(self)
    }
}

pub fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

pub fn is_kde_wayland() -> bool {
    is_wayland()
        && std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|p| p == "KDE"))
}

/// Ask KWin to place the window with app id `app_id` when it appears, and again whenever it
/// changes size (moves are left alone, so it can still be dragged). Without an anchor it goes
/// to the corner nearest the usual tray position (bottom right). Returns a guard that removes
/// the script again.
pub fn kwin_place(app_id: &str, anchor: Option<Anchor>, size: [f32; 2]) -> Option<ScriptGuard> {
    let (x, y) = match anchor.and_then(Anchor::known) {
        Some(a) => (a.x.to_string(), a.y.to_string()),
        None => ("-1".into(), "-1".into()),
    };
    let script = format!(
        r#"
const appId = "{app_id}";
const clickX = {x}, clickY = {y};
const margin = 8;
function place(w) {{
    if (!w || w.resourceClass != appId) return;
    const area = workspace.clientArea(KWin.PlacementArea, w);
    const width = w.frameGeometry.width || {width}, height = w.frameGeometry.height || {height};
    let px, py;
    if (clickX < 0) {{
        px = area.x + area.width - width - margin;
        py = area.y + area.height - height - margin;
    }} else {{
        px = clickX - width / 2;
        // Panel at the bottom (click in the lower half): open upwards, else downwards.
        py = clickY > area.y + area.height / 2 ? area.y + area.height - height - margin : area.y + margin;
    }}
    px = Math.max(area.x + margin, Math.min(px, area.x + area.width - width - margin));
    py = Math.max(area.y + margin, Math.min(py, area.y + area.height - height - margin));
    w.frameGeometry = {{ x: Math.round(px), y: Math.round(py), width: width, height: height }};
}}
function watch(w) {{
    if (!w || w.resourceClass != appId) return;
    place(w);
    let width = w.frameGeometry.width, height = w.frameGeometry.height;
    w.frameGeometryChanged.connect(function () {{
        const g = w.frameGeometry;
        if (g.width == width && g.height == height) return;
        width = g.width;
        height = g.height;
        place(w);
    }});
}}
// KWin 6 calls them windows, KWin 5 clients.
const added = workspace.windowAdded || workspace.clientAdded;
added.connect(watch);
"#,
        width = size[0],
        height = size[1],
    );

    let path = std::env::temp_dir().join(format!("{PLUGIN}-{}.js", std::process::id()));
    std::fs::write(&path, script).ok()?;
    let conn = zbus::blocking::Connection::session().ok()?;
    let scripting = zbus::blocking::Proxy::new(
        &conn,
        "org.kde.KWin",
        "/Scripting",
        "org.kde.kwin.Scripting",
    )
    .ok()?;
    // A previous popup that crashed may have left its script loaded.
    let _ = scripting.call::<_, _, bool>("unloadScript", &(PLUGIN,));
    let loaded =
        scripting.call::<_, _, i32>("loadScript", &(path.to_string_lossy().as_ref(), PLUGIN));
    if loaded.is_err() {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    let _ = scripting.call::<_, _, ()>("start", &());
    Some(ScriptGuard { conn, path })
}

pub struct ScriptGuard {
    conn: zbus::blocking::Connection,
    path: std::path::PathBuf,
}

impl Drop for ScriptGuard {
    fn drop(&mut self) {
        if let Ok(scripting) = zbus::blocking::Proxy::new(
            &self.conn,
            "org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting",
        ) {
            let _ = scripting.call::<_, _, bool>("unloadScript", &(PLUGIN,));
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Window position for X11, where apps may place themselves: centred on the click,
/// above it (the popup height is unknown to the tray host, so assume a bottom panel).
pub fn x11_position(anchor: Option<Anchor>, size: [f32; 2]) -> Option<[f32; 2]> {
    let a = anchor?.known()?;
    let x = (a.x as f32 - size[0] / 2.0).max(8.0);
    let y = (a.y as f32 - size[1] - 16.0).max(8.0);
    Some([x, y])
}
