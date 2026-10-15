//! Native file dialogs with fallbacks: XDG desktop portal (through rfd), then kdialog, then zenity.
//! Always call from a background thread; every function blocks until the user closes the dialog.
//! `Ok(None)` means the user cancelled, `Err` means no dialog could be shown.
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone)]
pub enum Req {
    Folders { title: String },
    Files { title: String, filter: (String, Vec<String>) },
    Open { title: String, filter: (String, Vec<String>) },
    Save { title: String, filter: (String, Vec<String>), name: String },
}

fn home() -> String { std::env::var("HOME").unwrap_or_else(|_| "/".into()) }

fn has(cmd: &str) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file())).unwrap_or(false)
}

/// Is a FileChooser portal reachable on the session bus? (3 s timeout)
pub fn portal_ok() -> Result<(), String> {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() && std::env::var_os("XDG_RUNTIME_DIR").is_none() { return Err("no D-Bus session bus".into()); }
    if !has("gdbus") { return Ok(()); } // can't check; let rfd try
    let o = Command::new("gdbus").args(["call", "--session", "--timeout", "3", "--dest", "org.freedesktop.portal.Desktop",
        "--object-path", "/org/freedesktop/portal/desktop", "--method", "org.freedesktop.DBus.Properties.Get",
        "org.freedesktop.portal.FileChooser", "version"]).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }
}

fn via_rfd(r: &Req) -> Result<Option<Vec<PathBuf>>, String> {
    let r = r.clone();
    let res = std::thread::spawn(move || {
        let flt = |d: rfd::FileDialog, f: &(String, Vec<String>)| if f.1.is_empty() { d } else { d.add_filter(&f.0, &f.1) };
        match r {
            Req::Folders { title } => rfd::FileDialog::new().set_title(&title).pick_folders(),
            Req::Files { title, filter } => flt(rfd::FileDialog::new().set_title(&title), &filter).pick_files(),
            Req::Open { title, filter } => flt(rfd::FileDialog::new().set_title(&title), &filter).pick_file().map(|p| vec![p]),
            Req::Save { title, filter, name } => flt(rfd::FileDialog::new().set_title(&title), &filter).set_file_name(&name).save_file().map(|p| vec![p]),
        }
    }).join();
    match res {
        Ok(v) => Ok(v),
        Err(e) => Err(e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "dialog crashed".into())),
    }
}

fn lines(o: std::process::Output) -> Result<Option<Vec<PathBuf>>, String> {
    if o.status.success() {
        let v: Vec<PathBuf> = String::from_utf8_lossy(&o.stdout).lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect();
        Ok(if v.is_empty() { None } else { Some(v) })
    } else if o.status.code() == Some(1) { Ok(None) } // cancelled
    else { Err(format!("exit {:?}: {}", o.status.code(), String::from_utf8_lossy(&o.stderr).trim())) }
}

fn via_kdialog(r: &Req) -> Result<Option<Vec<PathBuf>>, String> {
    let mut c = Command::new("kdialog");
    let kf = |f: &(String, Vec<String>)| if f.1.is_empty() { String::new() } else { format!("{} ({})", f.0, f.1.iter().map(|e| format!("*.{e}")).collect::<Vec<_>>().join(" ")) };
    match r {
        Req::Folders { title } => { c.args(["--title", title, "--getexistingdirectory", &home()]); }
        Req::Files { title, filter } => { c.args(["--title", title, "--multiple", "--separate-output", "--getopenfilename", &home(), &kf(filter)]); }
        Req::Open { title, filter } => { c.args(["--title", title, "--getopenfilename", &home(), &kf(filter)]); }
        Req::Save { title, filter, name } => { c.args(["--title", title, "--getsavefilename", &format!("{}/{}", home(), name), &kf(filter)]); }
    }
    lines(c.output().map_err(|e| e.to_string())?)
}

fn via_zenity(r: &Req) -> Result<Option<Vec<PathBuf>>, String> {
    let mut c = Command::new("zenity");
    c.args(["--file-selection", "--separator=\n"]);
    let zf = |c: &mut Command, f: &(String, Vec<String>)| { if !f.1.is_empty() { c.arg(format!("--file-filter={} | {}", f.0, f.1.iter().map(|e| format!("*.{e}")).collect::<Vec<_>>().join(" "))); } };
    match r {
        Req::Folders { title } => { c.args(["--directory", "--multiple", &format!("--title={title}")]); }
        Req::Files { title, filter } => { c.args(["--multiple", &format!("--title={title}")]); zf(&mut c, filter); }
        Req::Open { title, filter } => { c.arg(format!("--title={title}")); zf(&mut c, filter); }
        Req::Save { title, filter, name } => { c.args(["--save", "--confirm-overwrite", &format!("--title={title}"), &format!("--filename={}/{}", home(), name)]); zf(&mut c, filter); }
    }
    lines(c.output().map_err(|e| e.to_string())?)
}

/// Which backends would be tried, in order (for diagnostics: `mixstudio --dialog-check`).
pub fn backends() -> Vec<(&'static str, Result<(), String>)> {
    let forced = std::env::var("MIXSTUDIO_DIALOG").ok();
    let all = [("portal", portal_ok()),
        ("kdialog", if has("kdialog") { Ok(()) } else { Err("not installed".into()) }),
        ("zenity", if has("zenity") { Ok(()) } else { Err("not installed".into()) })];
    all.into_iter().filter(|(n, _)| forced.as_deref().map_or(true, |f| f == *n)).collect()
}

/// Show the dialog with the first backend that works.
pub fn run(r: Req) -> Result<Option<Vec<PathBuf>>, String> {
    let mut errs = vec![];
    for (name, ok) in backends() {
        if let Err(e) = ok { errs.push(format!("{name}: {e}")); continue; }
        let res = match name { "portal" => via_rfd(&r), "kdialog" => via_kdialog(&r), _ => via_zenity(&r) };
        match res {
            Ok(v) => return Ok(v),
            Err(e) => { eprintln!("file dialog via {name} failed: {e}"); errs.push(format!("{name}: {e}")); }
        }
    }
    Err(format!("no file dialog could be opened ({}). Type or paste the path instead, or drop it on the window.", errs.join("; ")))
}
