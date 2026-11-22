//! Debug information (spec §12.6): the `.dbg` files `juku-asm -g` writes,
//! loaded and indexed by address and by source line.

use std::ops::Range;
use std::path::{Component, Path, PathBuf};

use crate::json::Value;

/// Bytes made by one source line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub addr: u16,
    pub len: u16,
    /// Index into [`Info::files`].
    pub file: usize,
    pub line: u32,
}

impl Span {
    fn range(&self) -> Range<u16> {
        self.addr..self.addr.saturating_add(self.len)
    }
}

/// The debug information of the programs loaded, together.
#[derive(Default)]
pub(crate) struct Info {
    /// Source files, absolute.
    pub files: Vec<PathBuf>,
    /// Instructions: the spans of each line that makes code.
    code: Vec<Span>,
    /// For each address, 1 + the index of the code span holding it, or 0.
    code_at: Vec<u32>,
    /// For each address, whether a line of data (DB, DW, DS) made it.
    data_at: Vec<bool>,
    /// The sizes of labelled data: name and bytes.
    sizes: Vec<(String, u16)>,
    /// Labels, by address.
    labels: Vec<(u16, String)>,
    /// The first file's load address and entry.
    load: Option<u16>,
    entry: Option<u16>,
    /// Labels and EQUs, by value, labels first where both have it.
    names: Vec<(u16, bool, String)>,
}

/// A path with its `.` and `..` worked out, so that the client's spelling
/// of a file and the debug information's compare equal. Done by hand:
/// `fs::canonicalize` would also follow links, but brought 10 KB of the
/// standard library's formatting and file system code into the app.
pub(crate) fn normal_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir if out.file_name().is_some() => {
                out.pop();
            }
            // Above the root is the root.
            Component::ParentDir if out.has_root() => {}
            c => out.push(c),
        }
    }
    out
}

/// Inserts `item` into `list`, kept in the order `key` gives. A sort would
/// bring a generic sorting routine into the app for each kind of list.
fn insert_by<T, K: Ord>(list: &mut Vec<T>, item: T, key: impl Fn(&T) -> K) {
    let k = key(&item);
    let i = list.partition_point(|x| key(x) <= k);
    list.insert(i, item);
}

impl Info {
    /// Adds the `.dbg` file at `path`.
    pub(crate) fn load(&mut self, path: &Path) -> Result<(), String> {
        let shown = path.to_string_lossy().into_owned();
        let text = std::fs::read_to_string(path).map_err(|_| shown.clone() + " could not be read")?;
        self.load_text(&text, path.parent().unwrap_or(Path::new(".")), &shown)
    }

    /// Adds debug information given as its text: `folder` is where its
    /// paths start from, `shown` names it in errors.
    pub(crate) fn load_text(&mut self, text: &str, folder: &Path, shown: &str) -> Result<(), String> {
        let shown = shown.to_string();
        let file = Value::parse(text).map_err(|e| shown.clone() + ": " + &e)?;
        let bad = || shown.clone() + " is not debug information of this version";
        if file["version"].as_i64() != Some(2) {
            return Err(bad());
        }
        // Everything is read before anything is added, so a bad file adds
        // nothing.
        let mut files = Vec::new();
        for f in file["files"].as_array() {
            files.push(normal_path(&folder.join(f.as_str().ok_or_else(bad)?)));
        }
        let first_file = self.files.len();
        let number = |v: &Value| v.as_i64().and_then(|n| u32::try_from(n).ok());
        // Each row of `lines` is a run of code: address, file, first line,
        // its length, then for each line after it the step in lines and the
        // length.
        let mut code = Vec::new();
        for row in file["lines"].as_array() {
            let row: Vec<u32> = row.as_array().iter().map(number).collect::<Option<_>>().ok_or_else(bad)?;
            let [addr, f, line, len, rest @ ..] = &row[..] else { return Err(bad()) };
            if *f as usize >= files.len() || rest.len() % 2 != 0 {
                return Err(bad());
            }
            let (mut addr, mut line) = (*addr, *line);
            let mut len = *len;
            let mut steps = rest.chunks(2);
            loop {
                if addr + len > 0x10000 {
                    return Err(bad());
                }
                code.push(Span { addr: addr as u16, len: len as u16, file: first_file + *f as usize, line });
                let Some(&[step, next]) = steps.next() else { break };
                addr += len;
                line += step;
                len = next;
            }
        }
        let mut data = Vec::new();
        for row in file["data"].as_array() {
            let row: Vec<u32> = row.as_array().iter().map(number).collect::<Option<_>>().ok_or_else(bad)?;
            let [addr, len] = row[..] else { return Err(bad()) };
            data.push(addr..(addr + len).min(0x10000));
        }
        let mut names = Vec::new();
        let mut sizes = Vec::new();
        for (key, label) in [("labels", true), ("equs", false)] {
            for row in file[key].as_array() {
                let (name, rest) = match row.as_array() {
                    [name, rest @ ..] if !rest.is_empty() && rest.len() <= 2 => (name.as_str().ok_or_else(bad)?, rest),
                    _ => return Err(bad()),
                };
                let value = number(&rest[0]).ok_or_else(bad)?;
                names.push((value as u16, label, name.to_string()));
                if let Some(size) = rest.get(1).and_then(number) {
                    sizes.push((name.to_string(), size as u16));
                }
            }
        }

        self.files.extend(files);
        let address = |key: &str| file[key].as_i64().and_then(|v| u16::try_from(v).ok());
        self.load = self.load.or(address("load"));
        self.entry = self.entry.or(address("entry"));
        if self.code_at.is_empty() {
            self.code_at = vec![0; 0x10000];
            self.data_at = vec![false; 0x10000];
        }
        for range in data {
            self.data_at[range.start as usize..range.end as usize].fill(true);
        }
        self.sizes.extend(sizes);
        for s in code {
            self.code.push(s);
            for a in s.range() {
                self.code_at[a as usize] = self.code.len() as u32;
            }
        }
        for n in names {
            if n.1 {
                insert_by(&mut self.labels, (n.0, n.2.clone()), |l| l.0);
            }
            // Labels before EQUs of the same value.
            insert_by(&mut self.names, n, |n| (n.0, !n.1));
        }
        Ok(())
    }

    fn span_at(&self, addr: u16) -> Option<&Span> {
        let i = *self.code_at.get(addr as usize)?;
        (i > 0).then(|| &self.code[i as usize - 1])
    }

    /// The source file and line whose code holds `addr`.
    pub(crate) fn source_line(&self, addr: u16) -> Option<(&Path, u32)> {
        self.span_at(addr).map(|s| (self.files[s.file].as_path(), s.line))
    }

    /// A line's instruction starts at `addr`.
    pub(crate) fn line_start(&self, addr: u16) -> bool {
        self.span_at(addr).is_some_and(|s| s.addr == addr)
    }

    /// A line of data made `addr`.
    pub(crate) fn data(&self, addr: u16) -> bool {
        self.data_at.get(addr as usize).copied().unwrap_or(false)
    }

    /// The code of the line holding `addr`, all of it: a line in a `REPT`
    /// block has a span for each time round.
    pub(crate) fn line_ranges(&self, addr: u16) -> Vec<Range<u16>> {
        let Some(&at) = self.span_at(addr) else {
            return Vec::new();
        };
        self.code.iter().filter(|s| s.file == at.file && s.line == at.line).map(Span::range).collect()
    }

    /// Where a breakpoint on `line` of `file` goes: the line itself if it
    /// makes code, else the next one in the file that does. The line found
    /// and the address of each of its spans.
    pub(crate) fn breakpoint(&self, file: &Path, line: u32) -> Option<(u32, Vec<u16>)> {
        let found = self
            .code
            .iter()
            .filter(|s| self.files[s.file] == file && s.line >= line)
            .map(|s| s.line)
            .min()?;
        let addrs = self.code.iter().filter(|s| self.files[s.file] == file && s.line == found).map(|s| s.addr).collect();
        Some((found, addrs))
    }

    /// The label at `addr`.
    pub(crate) fn label(&self, addr: u16) -> Option<&str> {
        let i = self.labels.partition_point(|l| l.0 < addr);
        self.labels.get(i).filter(|l| l.0 == addr).map(|l| l.1.as_str())
    }

    /// `addr` as a label and an offset (`DRAW.LOOP+3`), for an address in a
    /// program's code or data.
    pub(crate) fn place_name(&self, addr: u16) -> Option<String> {
        if self.span_at(addr).is_none() && !self.data(addr) {
            return None;
        }
        let i = self.labels.partition_point(|l| l.0 <= addr);
        let (at, name) = self.labels.get(i.checked_sub(1)?)?;
        Some(match addr - at {
            0 => name.clone(),
            n => name.clone() + "+" + &crate::text::dec(n as i64),
        })
    }

    /// A name for the value `v`: a label, or with `any`, an EQU too.
    pub(crate) fn name(&self, v: u16, any: bool) -> Option<&str> {
        let i = self.names.partition_point(|n| n.0 < v);
        self.names.get(i).filter(|n| n.0 == v && (n.1 || any)).map(|n| n.2.as_str())
    }

    /// The value of the symbol `name`. Names are case-sensitive, but one
    /// typed in another case is found too, when it matches one name only:
    /// `print` for `PRINT`, `conch` for `CONCH`. Two or more are an error.
    pub(crate) fn value(&self, name: &str) -> Result<Option<u16>, String> {
        if let Some(n) = self.names.iter().find(|n| n.2 == name) {
            return Ok(Some(n.0));
        }
        let mut like: Vec<&(u16, bool, String)> = Vec::new();
        for n in self.names.iter().filter(|n| n.2.eq_ignore_ascii_case(name)) {
            if !like.iter().any(|l| l.2 == n.2) {
                like.push(n);
            }
        }
        match like[..] {
            [] => Ok(None),
            [n] => Ok(Some(n.0)),
            _ => {
                let names: Vec<&str> = like.iter().map(|n| n.2.as_str()).collect();
                Err("“".to_string() + name + "” could be " + &names.join(" or "))
            }
        }
    }

    /// The labels, by address.
    pub(crate) fn labels(&self) -> &[(u16, String)] {
        &self.labels
    }

    /// The bytes the line of data at the label `name` makes: a variable's
    /// size.
    pub(crate) fn size(&self, name: &str) -> Option<u16> {
        self.sizes.iter().find(|s| s.0 == name).map(|s| s.1)
    }

    /// The load address of the first program loaded.
    pub(crate) fn load_address(&self) -> Option<u16> {
        self.load
    }

    /// The entry of the first program loaded.
    pub(crate) fn entry(&self) -> Option<u16> {
        self.entry
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.files.is_empty() && self.names.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(name: &str, text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("juku-dap-info-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    const DBG: &str = r#"{
      "version": 2, "image": "p.bin", "load": 256, "size": 9, "entry": 256,
      "files": ["p.asm"],
      "lines": [[256, 0, 4, 3, 2, 1, 3, 1, 0, 1]],
      "data": [[262, 3]],
      "labels": [["MAIN", 256], ["MAIN.LOOP", 259], ["MSG", 262, 3]],
      "equs": [["TTO", 65497], ["FIVE", 259]]
    }"#;

    #[test]
    fn lookups() {
        write("p.asm", "");
        let path = write("p.dbg", DBG);
        let mut info = Info::default();
        info.load(&path).unwrap();
        let asm = normal_path(&path.with_file_name("p.asm"));
        assert_eq!(info.source_line(0x0101), Some((asm.as_path(), 4)));
        assert!(info.line_start(0x0100) && !info.line_start(0x0101));
        assert!(info.data(0x0107) && !info.data(0x0103));
        // A REPT line: two spans.
        assert_eq!(info.line_ranges(0x0104), vec![0x0104..0x0105, 0x0105..0x0106]);
        // Line 5 makes no code: the breakpoint moves to line 6.
        assert_eq!(info.breakpoint(&asm, 5), Some((6, vec![0x0103])));
        assert_eq!(info.breakpoint(&asm, 9), Some((9, vec![0x0104, 0x0105])));
        assert_eq!(info.breakpoint(&asm, 13), None);
        assert_eq!(info.place_name(0x0105), Some("MAIN.LOOP+2".to_string()));
        assert_eq!(info.place_name(0x0108), Some("MSG+2".to_string()));
        assert_eq!(info.place_name(0x0200), None);
        assert_eq!(info.label(0x0103), Some("MAIN.LOOP"));
        assert_eq!((info.size("MSG"), info.size("MAIN")), (Some(3), None));
        assert_eq!(info.name(0x0103, true), Some("MAIN.LOOP"), "a label before an EQU");
        assert_eq!(info.name(0xFFD9, false), None);
        assert_eq!(info.name(0xFFD9, true), Some("TTO"));
    }

    /// Names are case-sensitive; one in another case is found if it is the
    /// only one that matches.
    #[test]
    fn names_in_any_case() {
        let path = write(
            "case.dbg",
            r#"{"version": 2, "labels": [["count", 256], ["main.loop", 259]], "equs": [["COUNT", 3], ["TTO", 65497]]}"#,
        );
        let mut info = Info::default();
        info.load(&path).unwrap();
        assert_eq!(info.value("count"), Ok(Some(256)));
        assert_eq!(info.value("COUNT"), Ok(Some(3)));
        assert!(info.value("Count").is_err(), "count or COUNT");
        assert_eq!(info.value("MAIN.LOOP"), Ok(Some(259)));
        assert_eq!(info.value("tto"), Ok(Some(65497)));
        assert_eq!(info.value("nosuch"), Ok(None));
    }

    #[test]
    fn paths() {
        assert_eq!(normal_path(Path::new("/a/b/./../c/d.asm")), Path::new("/a/c/d.asm"));
        assert_eq!(normal_path(Path::new("/a/../../b")), Path::new("/b"));
        assert_eq!(normal_path(Path::new("a/../../b")), Path::new("../b"));
    }

    #[test]
    fn refused() {
        let mut info = Info::default();
        assert!(info.load(&write("v1.dbg", r#"{"version": 1}"#)).is_err());
        assert!(info.load(&write("bad.dbg", r#"{"version": 2, "files": ["a"], "lines": [[1, 5, 3, 2]]}"#)).is_err());
        assert!(info.load(&write("odd.dbg", r#"{"version": 2, "files": ["a"], "lines": [[1, 0, 3, 2, 1]]}"#)).is_err());
        assert!(info.load(&write("big.dbg", r#"{"version": 2, "files": ["a"], "lines": [[65535, 0, 3, 2]]}"#)).is_err());
        assert!(info.load(&write("sym.dbg", r#"{"version": 2, "labels": [["A"]]}"#)).is_err());
        assert!(info.load(Path::new("/nonexistent/x.dbg")).is_err());
        assert!(info.is_empty());
    }
}
