//! The XDG side of the launcher: where desktop entries live, what a
//! `[Desktop Entry]` section says, and how a click becomes a running
//! application.
//!
//! Nothing here draws. Everything here is testable without a window,
//! which is the point of the split — the parsing rules of the Desktop
//! Entry Specification are fiddly enough to deserve tests, and the
//! launch path is the one place in this widget that can leave something
//! behind in the process table.

use std::path::{Path, PathBuf};

/// One installed application, as its desktop entry describes it.
#[derive(Clone, Debug)]
pub struct AppEntry {
    /// The desktop file ID: `firefox.desktop`, or `kde-konsole.desktop`
    /// for a file one directory down. This is what a user's copy
    /// shadows a system one BY — not the path, not the name.
    pub id: String,
    /// `Name`, in the running locale when the file offers one.
    pub name: String,
    /// `Exec`, verbatim. The field codes are expanded at launch, not
    /// here, so what the file said survives in the log line.
    pub exec: String,
    /// `Terminal=true`: the program wants a terminal window of its own.
    pub terminal: bool,
    /// `Icon`. Read because the entry HAS one and throwing it away here
    /// would mean parsing again later; nothing draws it, because the
    /// project has no icon theme yet.
    pub icon: String,
    /// `Categories`, split on `;`. Kept for the same reason: the
    /// grouping this grid does not do yet is a filter over this field.
    pub categories: Vec<String>,
}

/// An environment variable that is set AND not empty. The spec treats
/// an empty `XDG_DATA_DIRS` as unset, and so does everything else here.
fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// Every `applications` directory, in the specification's precedence
/// order: `$XDG_DATA_HOME` first, then each of `$XDG_DATA_DIRS`. The
/// defaults are the spec's own (`~/.local/share`, and
/// `/usr/local/share:/usr/share`). Relative entries are ignored, which
/// the base-directory spec requires.
pub fn applications_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let data_home = env_nonempty("XDG_DATA_HOME").map(PathBuf::from).or_else(|| {
        env_nonempty("HOME").map(|h| PathBuf::from(h).join(".local/share"))
    });
    if let Some(d) = data_home.filter(|d| d.is_absolute()) {
        out.push(d.join("applications"));
    }
    let dirs =
        env_nonempty("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    for d in dirs.split(':').map(PathBuf::from).filter(|d| d.is_absolute()) {
        let d = d.join("applications");
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

/// The locale keys `Name[...]` is looked up under, best match first.
///
/// The spec's own order for a `lang_COUNTRY.ENCODING@MODIFIER` locale:
/// `lang_COUNTRY@MODIFIER`, `lang_COUNTRY`, `lang@MODIFIER`, `lang` —
/// the encoding never takes part. `C` and `POSIX` mean "no
/// translation", so they answer nothing and the plain `Name` wins.
pub fn locale_candidates() -> Vec<String> {
    let raw = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|v| env_nonempty(v))
        .unwrap_or_default();
    locale_keys(&raw)
}

/// [`locale_candidates`]'s arithmetic, on a locale given rather than
/// read — the half that can be tested.
pub fn locale_keys(raw: &str) -> Vec<String> {
    if raw.is_empty() || raw == "C" || raw == "POSIX" {
        return Vec::new();
    }
    let (head, modifier) = match raw.split_once('@') {
        Some((h, m)) => (h, Some(m)),
        None => (raw, None),
    };
    let head = head.split('.').next().unwrap_or(head);
    let (lang, country) = match head.split_once('_') {
        Some((l, c)) => (l, Some(c)),
        None => (head, None),
    };
    let mut out = Vec::new();
    match (country, modifier) {
        (Some(c), Some(m)) => {
            out.push(format!("{lang}_{c}@{m}"));
            out.push(format!("{lang}_{c}"));
            out.push(format!("{lang}@{m}"));
        }
        (Some(c), None) => out.push(format!("{lang}_{c}")),
        (None, Some(m)) => out.push(format!("{lang}@{m}")),
        (None, None) => {}
    }
    out.push(lang.to_string());
    out
}

/// A string value's escape sequences, as the spec spells them.
fn unescape(v: &str) -> String {
    if !v.contains('\\') {
        return v.to_string();
    }
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// One desktop file's `[Desktop Entry]` group, or None when it is not
/// an application this launcher may show.
///
/// The filters are the spec's: `Type` must be `Application`, `Hidden`
/// means the entry was deleted, `NoDisplay` means it exists but is not
/// for a menu, and `TryExec` naming a program that is not on `PATH`
/// means the application is not actually installed.
pub fn parse(text: &str, locales: &[String], id: String) -> Option<AppEntry> {
    let mut in_entry = false;
    let (mut ty, mut name, mut exec) = (String::new(), String::new(), String::new());
    let (mut icon, mut try_exec) = (String::new(), String::new());
    let (mut terminal, mut no_display, mut hidden) = (false, false, false);
    let mut categories: Vec<String> = Vec::new();
    let mut localised: Vec<(String, String)> = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(group) = line.strip_prefix('[') {
            // A second `[Desktop Entry]` is not legal and not defended
            // against: entering the group again would simply overwrite.
            in_entry = group.trim_end_matches(']') == "Desktop Entry";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        let (key, loc) = match k.split_once('[') {
            Some((key, rest)) => (key.trim(), rest.strip_suffix(']')),
            None => (k, None),
        };
        match (key, loc) {
            ("Type", None) => ty = v.to_string(),
            ("Name", None) => name = unescape(v),
            ("Name", Some(l)) => localised.push((l.to_string(), unescape(v))),
            ("Exec", None) => exec = v.to_string(),
            ("Icon", None) => icon = v.to_string(),
            ("TryExec", None) => try_exec = v.to_string(),
            ("Terminal", None) => terminal = v == "true",
            ("NoDisplay", None) => no_display = v == "true",
            ("Hidden", None) => hidden = v == "true",
            ("Categories", None) => {
                categories =
                    v.split(';').filter(|s| !s.is_empty()).map(str::to_string).collect()
            }
            _ => {}
        }
    }

    if ty != "Application" || hidden || no_display || exec.is_empty() {
        return None;
    }
    if !try_exec.is_empty() && which(&try_exec).is_none() {
        return None;
    }
    // The best locale that the file actually offers, then the
    // untranslated Name, then — for a file so broken it has neither —
    // its own id, because a tile with no caption is worse than a tile
    // captioned by its filename.
    let display = locales
        .iter()
        .find_map(|l| localised.iter().find(|(k, _)| k == l).map(|(_, v)| v.clone()))
        .filter(|s| !s.is_empty())
        .or(Some(name).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| id.trim_end_matches(".desktop").to_string());

    Some(AppEntry { id, name: display, exec, terminal, icon, categories })
}

/// Every `<dir>/**/*.desktop` under one applications directory, as
/// (id, path) pairs. The id is the path relative to the directory with
/// `/` turned into `-`, which is the spec's own rule.
fn collect(root: &Path, dir: &Path, depth: u32, out: &mut Vec<(String, PathBuf)>) {
    // A cap rather than a visited set: a symlink loop is the only way
    // to go deep, and no real menu nests more than two directories.
    if depth > 6 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        let Ok(ft) = e.file_type() else { continue };
        // Follow links: a distribution that symlinks a directory of
        // entries into place means them to be found.
        let is_dir = std::fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(ft.is_dir());
        if is_dir {
            collect(root, &path, depth + 1, out);
            continue;
        }
        if path.extension().and_then(|x| x.to_str()) != Some("desktop") {
            continue;
        }
        let Some(rel) = path.strip_prefix(root).ok().and_then(|p| p.to_str()) else {
            continue;
        };
        out.push((rel.replace('/', "-"), path));
    }
}

/// The installed applications, sorted by the name they display under.
pub fn scan() -> Vec<AppEntry> {
    scan_dirs(&applications_dirs(), &locale_candidates())
}

/// [`scan`] over directories given rather than read from the
/// environment — the same function, minus the two environment lookups,
/// so the precedence rule can be tested without touching the
/// process's own environment.
pub fn scan_dirs(dirs: &[PathBuf], locales: &[String]) -> Vec<AppEntry> {
    let mut taken: Vec<String> = Vec::new();
    let mut out: Vec<AppEntry> = Vec::new();
    for dir in dirs {
        let mut files = Vec::new();
        collect(dir, dir, 0, &mut files);
        // Sorted, so which of two files in ONE directory wins never
        // depends on the order the filesystem hands them back.
        files.sort();
        for (id, path) in files {
            if taken.contains(&id) {
                continue;
            }
            // The id is claimed by the first directory that holds it
            // whether or not the entry survives the filters: a user's
            // `Hidden=true` copy is the spec's way of DELETING a system
            // entry, and it can only do that by being counted here.
            taken.push(id.clone());
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            if let Some(e) = parse(&text, locales, id) {
                out.push(e);
            }
        }
    }
    out.sort_by(|a, b| {
        a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id))
    });
    out
}

/// A cheap number that changes when the menu does: the modification
/// times of the applications directories themselves, folded together.
/// Installing or removing an application rewrites the directory it
/// lives in, so this catches every case a launcher cares about without
/// walking a single file.
pub fn stamp() -> u64 {
    let mut acc: u64 = 0;
    for dir in applications_dirs() {
        let secs = std::fs::metadata(&dir)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        acc = acc.wrapping_mul(31).wrapping_add(secs);
    }
    acc
}

/// The program on `PATH`, or the path itself when the name has a `/`.
pub fn which(prog: &str) -> Option<PathBuf> {
    if prog.is_empty() {
        return None;
    }
    if prog.contains('/') {
        let p = PathBuf::from(prog);
        return executable(&p).then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.join(prog))
        .find(|c| executable(c))
}

fn executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// The field codes of the specification. `%f %F %u %U` want files this
/// launcher never has, `%i %c %k` want the entry's own icon, name and
/// path, and `%d %D %n %N %v %m` are deprecated. Every one of them
/// comes OUT of the command line; `%%` collapses to a single `%`; a
/// code this list does not know survives verbatim, because guessing at
/// it would be worse than passing it on.
const FIELD_CODES: &str = "fFuUickdDnNvm";

/// One `Exec` value as an argument vector.
///
/// Two steps, in the spec's own order: split on whitespace honouring
/// double quotes (inside which `\\`, `\"`, `` \` `` and `\$` are the
/// escapes), then take the field codes out of each argument. An
/// argument that was NOTHING BUT a field code disappears — `foo %U`
/// runs `foo`, with no stray empty argument after it — while an
/// argument that was written empty on purpose (`""`) stays.
pub fn expand_exec(exec: &str) -> Vec<String> {
    split_argv(exec)
        .into_iter()
        .filter_map(|tok| {
            let (s, dropped) = strip_field_codes(&tok);
            (!(s.is_empty() && dropped)).then_some(s)
        })
        .collect()
}

fn split_argv(exec: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut it = exec.chars();
    while let Some(c) = it.next() {
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            '"' => {
                // A quoted run is part of the argument it touches, so
                // `--title"a b"` is one argument, as a shell would say.
                // The four escapes the spec allows are read HERE rather
                // than in a second pass, because a `\"` must not be
                // mistaken for the quote that ends the run.
                started = true;
                while let Some(q) = it.next() {
                    match q {
                        '\\' => match it.next() {
                            Some(e @ ('"' | '`' | '$' | '\\')) => cur.push(e),
                            Some(other) => {
                                cur.push('\\');
                                cur.push(other);
                            }
                            None => cur.push('\\'),
                        },
                        '"' => break,
                        other => cur.push(other),
                    }
                }
            }
            other => {
                started = true;
                cur.push(other);
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

/// The argument without its field codes, and whether any were taken out.
fn strip_field_codes(tok: &str) -> (String, bool) {
    if !tok.contains('%') {
        return (tok.to_string(), false);
    }
    let mut out = String::with_capacity(tok.len());
    let mut dropped = false;
    let mut it = tok.chars();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('%') => out.push('%'),
            Some(code) if FIELD_CODES.contains(code) => dropped = true,
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    (out, dropped)
}

/// A `Terminal=true` entry's command line, wrapped in whatever terminal
/// this system has: `$TERMINAL` when the user named one, otherwise the
/// `x-terminal-emulator` alternative. Neither installed = None, and the
/// caller says so rather than starting something invisible.
fn in_terminal(argv: &[String]) -> Option<Vec<String>> {
    let named = env_nonempty("TERMINAL");
    let term = named
        .as_deref()
        .filter(|t| which(t).is_some())
        .or(Some("x-terminal-emulator").filter(|t| which(t).is_some()))?;
    let mut out = vec![term.to_string(), "-e".to_string()];
    out.extend_from_slice(argv);
    Some(out)
}

/// Runs the entry, detached, and says so on stderr.
///
/// Detached means what it says: the application must survive the
/// desktop that started it. It is forked twice with a `setsid` between
/// — session of its own, so a terminal signal cannot reach it, and an
/// orphan, so init reaps it rather than this process — and the
/// intermediate is waited for here, which is what keeps a zombie out of
/// the table.
pub fn launch(app: &AppEntry) -> Result<(), String> {
    let argv = expand_exec(&app.exec);
    if argv.is_empty() {
        return Err(format!("Exec has no command: {:?}", app.exec));
    }
    let argv = if app.terminal {
        match in_terminal(&argv) {
            Some(v) => v,
            None => {
                return Err(
                    "wants a terminal, and neither $TERMINAL nor x-terminal-emulator \
                     is installed"
                        .to_string(),
                )
            }
        }
    } else {
        argv
    };
    spawn_detached(&argv).map(|_| {
        eprintln!("appgrid: launched {} \u{2014} {}", app.name, argv.join(" "));
    })
}

fn spawn_detached(argv: &[String]) -> Result<(), String> {
    // Everything that allocates happens HERE, before the fork: between
    // fork and exec a process with more than one thread may call only
    // async-signal-safe functions, and the allocator is not one.
    let mut owned = Vec::with_capacity(argv.len());
    for a in argv {
        owned.push(
            std::ffi::CString::new(a.as_bytes())
                .map_err(|_| format!("argument contains a NUL byte: {a:?}"))?,
        );
    }
    let mut ptrs: Vec<*const libc::c_char> = owned.iter().map(|c| c.as_ptr()).collect();
    ptrs.push(std::ptr::null());

    unsafe {
        match libc::fork() {
            -1 => Err(format!("fork failed: {}", std::io::Error::last_os_error())),
            0 => {
                libc::setsid();
                if libc::fork() == 0 {
                    libc::execvp(ptrs[0], ptrs.as_ptr());
                    // exec only returns on failure, and the parent is
                    // long gone: 127 is the shell's own code for it.
                    libc::_exit(127);
                }
                libc::_exit(0);
            }
            pid => {
                // The intermediate exits immediately; reaping it is the
                // whole reason it exists. The application itself is now
                // init's child and none of ours.
                let mut status: libc::c_int = 0;
                while libc::waitpid(pid, &mut status, 0) < 0
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
                {}
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_loses_its_field_codes_and_keeps_its_quotes() {
        // The ordinary case: a code at the end takes its whole argument
        // with it rather than leaving an empty one behind.
        assert_eq!(expand_exec("firefox %u"), ["firefox"]);
        assert_eq!(expand_exec("/usr/bin/foo --flag %F"), ["/usr/bin/foo", "--flag"]);
        // Every code the spec defines, deprecated ones included.
        assert_eq!(expand_exec("a %f %F %u %U %i %c %k %d %D %n %N %v %m b"), ["a", "b"]);
        // Quoting: a space inside quotes is not a separator, and a
        // quoted run joins the argument it touches.
        assert_eq!(
            expand_exec("\"/opt/my app/bin\" --title \"Hello World\" %i"),
            ["/opt/my app/bin", "--title", "Hello World"]
        );
        assert_eq!(expand_exec("--title=\"a b\""), ["--title=a b"]);
        // The escapes that are legal inside a quoted run.
        assert_eq!(expand_exec(r#"sh -c "echo \"hi\" \\ \$HOME""#), [
            "sh",
            "-c",
            r#"echo "hi" \ $HOME"#
        ]);
        // %% is one per cent, and a code nobody knows survives.
        assert_eq!(expand_exec("meter 100%% %z"), ["meter", "100%", "%z"]);
        // A code in the MIDDLE of an argument goes without taking the
        // argument with it.
        assert_eq!(expand_exec("foo pre%fpost"), ["foo", "prepost"]);
        // An argument written empty on purpose is not a dropped code.
        assert_eq!(expand_exec(r#"foo "" bar"#), ["foo", "", "bar"]);
        assert!(expand_exec("   ").is_empty());
    }

    #[test]
    fn the_name_comes_from_the_best_locale_the_file_offers() {
        let text = "\
[Desktop Entry]
Type=Application
Name=Text Editor
Name[pl]=Edytor
Name[pl_PL]=Edytor tekstu
Name[de]=Texteditor
Exec=gedit %U
";
        let pick = |raw: &str| {
            parse(text, &locale_keys(raw), "gedit.desktop".to_string()).unwrap().name
        };
        // The full key beats the language alone.
        assert_eq!(pick("pl_PL.UTF-8"), "Edytor tekstu");
        // A country the file does not translate falls back to the
        // language, not to the untranslated name.
        assert_eq!(pick("pl_BR.UTF-8"), "Edytor");
        assert_eq!(pick("de_AT.UTF-8@euro"), "Texteditor");
        // A language the file does not know, and the C locale, both get
        // the plain Name.
        assert_eq!(pick("fi_FI.UTF-8"), "Text Editor");
        assert_eq!(pick("C"), "Text Editor");
        assert_eq!(pick(""), "Text Editor");
        // The candidate list itself, in the spec's order.
        assert_eq!(locale_keys("sr_RS.UTF-8@latin"), [
            "sr_RS@latin",
            "sr_RS",
            "sr@latin",
            "sr"
        ]);
    }

    #[test]
    fn the_entries_a_menu_must_not_show_are_filtered_out() {
        let with = |extra: &str| {
            let text = format!(
                "[Desktop Entry]\nType=Application\nName=Thing\nExec=thing\n{extra}"
            );
            parse(&text, &[], "thing.desktop".to_string()).is_some()
        };
        assert!(with(""), "an ordinary entry shows");
        assert!(!with("NoDisplay=true\n"));
        assert!(!with("Hidden=true\n"));
        assert!(with("NoDisplay=false\nHidden=false\n"));
        // TryExec: a program that is not there means the application is
        // not installed, whatever Exec says.
        assert!(!with("TryExec=/nonexistent/definitely-not-here\n"));
        assert!(with("TryExec=sh\n"));
        // Only Application; a Link or a Directory is not launchable.
        for ty in ["Link", "Directory", ""] {
            let text = format!("[Desktop Entry]\nType={ty}\nName=Thing\nExec=thing\n");
            assert!(parse(&text, &[], "t.desktop".to_string()).is_none(), "Type={ty}");
        }
        // Keys outside the [Desktop Entry] group belong to their own
        // group and must not leak into it.
        let actions = "\
[Desktop Entry]
Type=Application
Name=Thing
Exec=thing
Terminal=false

[Desktop Action New]
Name=New Window
Exec=thing --new
Terminal=true
";
        let e = parse(actions, &[], "thing.desktop".to_string()).unwrap();
        assert_eq!(e.name, "Thing");
        assert_eq!(e.exec, "thing");
        assert!(!e.terminal, "the action group's Terminal is not the entry's");
        // Everything else the parser is asked for.
        let full = "\
[Desktop Entry]
Type=Application
Name=Files\\sand\\sthings
Exec=nautilus %U
Icon=org.gnome.Nautilus
Terminal=true
Categories=System;FileTools;
";
        let e = parse(full, &[], "nautilus.desktop".to_string()).unwrap();
        assert_eq!(e.name, "Files and things", "\\s is a space");
        assert_eq!(e.icon, "org.gnome.Nautilus");
        assert!(e.terminal);
        assert_eq!(e.categories, ["System", "FileTools"]);
    }

    #[test]
    fn a_users_copy_shadows_the_system_one_by_id() {
        let base = std::env::temp_dir()
            .join(format!("nacelle-appgrid-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (user, system) = (base.join("user"), base.join("system"));
        let write = |root: &Path, rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        let app = |name: &str| {
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n")
        };
        // Same id in both roots: the user's wins.
        write(&user, "editor.desktop", &app("User Editor"));
        write(&system, "editor.desktop", &app("System Editor"));
        // A user entry that only DELETES a system one: the id is taken,
        // and nothing is shown for it.
        write(
            &user,
            "browser.desktop",
            "[Desktop Entry]\nType=Application\nName=Browser\nExec=/bin/true\nHidden=true\n",
        );
        write(&system, "browser.desktop", &app("System Browser"));
        // A subdirectory's id carries the directory name.
        write(&system, "kde/konsole.desktop", &app("Konsole"));
        // System-only, and a file that is not an entry at all.
        write(&system, "calc.desktop", &app("Calculator"));
        write(&system, "mimeinfo.cache", "not an entry");

        let found = scan_dirs(&[user.clone(), system.clone()], &[]);
        let names: Vec<&str> = found.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            ["Calculator", "Konsole", "User Editor"],
            "sorted by name; the user's editor shadows the system's, and its \
             Hidden browser deletes the system's"
        );
        assert_eq!(
            found.iter().find(|e| e.name == "Konsole").unwrap().id,
            "kde-konsole.desktop"
        );
        // The other precedence direction, to be sure the order of the
        // directories is what decides and nothing else.
        let found = scan_dirs(&[system, user], &[]);
        assert!(found.iter().any(|e| e.name == "System Editor"));
        assert!(found.iter().any(|e| e.name == "System Browser"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_search_path_is_the_specifications() {
        let dirs = applications_dirs();
        assert!(dirs.iter().all(|d| d.ends_with("applications")));
        assert!(
            dirs.iter().all(|d| d.is_absolute()),
            "a relative XDG_DATA_DIRS entry is ignored"
        );
        // which() agrees with the shell about a program that must exist.
        assert!(which("sh").is_some());
        assert!(which("nacelle-definitely-no-such-program").is_none());
        assert!(which("/bin/sh").is_some() || which("/usr/bin/sh").is_some());
        assert!(which("/etc/hostname").is_none(), "a file is not an executable");
        assert!(which("").is_none());
    }
}
